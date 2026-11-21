import { ref, computed, watch } from 'vue';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';

import type { PdfPageData, SentenceBlock, ReadingMode, AppTheme } from '../types/pdf';

// Global application state
const pdfPath = ref<string>('');
const pdfUrl = ref<string>('');
const pdfPages = ref<PdfPageData[]>([]);
const currentSentenceIndex = ref<number>(-1);
const currentPageIndex = ref<number>(0);
const isLoading = ref<boolean>(false);
const isPlaying = ref<boolean>(false);
const isGeneratingAudio = ref<boolean>(false);
const selectedVoice = ref<string>('kokoro_af_heart');
const speechSpeed = ref<number>(1.0);
const zoomLevel = ref<number>(1.0);
const theme = ref<AppTheme>('light');
const readingMode = ref<ReadingMode>('clean');
const skipHeadersFooters = ref<boolean>(true);
const isSidebarOpen = ref<boolean>(false);
const activeSidebarTab = ref<'transcript' | 'pages'>('transcript');
const isExportingAudio = ref<boolean>(false);
const exportProgress = ref<{ current: number; total: number }>({ current: 0, total: 0 });

interface PausedPlayback {
  pageIndex: number;
  sentenceIndex: number;
  chunkIndex: number;
  chunks: string[];
  voice: string;
  readingMode: ReadingMode;
}

const MAX_AUDIO_CACHE_ENTRIES = 32;
let currentAudio: HTMLAudioElement | null = null;
let pausedPlayback: PausedPlayback | null = null;
const audioCache = ref<Map<string, string>>(new Map());
const inFlightAudio = new Map<string, Promise<string | null>>();
let playbackSessionId = 0;

const ABBREVIATIONS = new Set([
  'e.g.', 'i.e.', 'etc.', 'vs.', 'cf.', 'al.', 'dr.', 'prof.', 'mr.', 'mrs.', 'ms.',
  'st.', 'fig.', 'figs.', 'eq.', 'eqs.', 'sec.', 'ch.', 'vol.', 'no.', 'pp.', 'p.',
  'jan.', 'feb.', 'mar.', 'apr.', 'aug.', 'sep.', 'sept.', 'oct.', 'nov.', 'dec.'
]);

function isParentheticalCitation(inner: string): boolean {
  const trimmed = inner.trim();
  if (!trimmed) return false;

  const parts = trimmed.split(';');
  if (parts.length > 12) return false;

  return parts.every((rawPart) => {
    const part = rawPart
      .trim()
      .replace(/^(?:see|e\.g\.|i\.e\.|cf\.)\s*,?\s*/i, '');
    if (!part || part.length > 80) return false;
    // Must start with an uppercase author name and end with a 4-digit year (plus optional page range)
    return /^[A-Z][^\n]{1,65}\b(?:19|20)\d{2}[a-z]?(?:,\s*pp?\.\s*\d+(?:[-–]\d+)?)?$/.test(part);
  });
}

/**
 * Strips academic clutter (in-text citations, LaTeX formula noise, hyphenation)
 * so neural TTS sounds natural and fluid.
 */
export function cleanAcademicText(raw: string): string {
  let cleaned = raw
    // Join hyphenated words split across lines/spaces: "infor- mation" -> "information"
    .replace(/([\p{L}\d])[-\u2010\u00AD]\s+([\p{L}])/gu, '$1$2')
    // Strip any leftover invisible soft hyphens
    .replace(/\u00AD/g, '')
    // Remove bracketed citations: "[1]", "[1, 2]", "[12-15]", "[4; 7]"
    .replace(/\[\s*\d+(?:[\s,;–-]+\d+)*\s*\]/g, '')
    // Remove complex author-date citations without nested quantifier backtracking: "(Smith et al., 2020)"
    .replace(/\(([^()]{1,240})\)/g, (full, inner: string) => (isParentheticalCitation(inner) ? '' : full))
    // Remove leftover isolated author years: "(2020)", "(1998, 2004)"
    .replace(/\(\s*\d{4}(?:,\s*\d{4})*\s*\)/g, '')
    // Remove raw URL references
    .replace(/https?:\/\/\S+/gi, '')
    // Remove extra spaces left before punctuation after citation removal
    .replace(/\s+([.,!?;:])/g, '$1')
    // Replace multiple spaces with a single space
    .replace(/\s+/g, ' ')
    .trim();

  // If cleaning stripped almost everything (e.g. line was only a citation), return original
  return cleaned.length > 2 ? cleaned : raw;
}

/**
 * Splits a single PDF line into sentence segments when punctuation appears mid-line,
 * avoiding false splits on abbreviations and initials.
 */
function splitLineIntoSegments(lineText: string): string[] {
  const segments: string[] = [];
  let start = 0;

  for (let i = 0; i < lineText.length; i++) {
    const ch = lineText[i];
    if (ch !== '.' && ch !== '!' && ch !== '?') continue;

    let endIdx = i + 1;
    while (endIdx < lineText.length && /["')\]]/.test(lineText[endIdx])) {
      endIdx++;
    }

    // Must be followed by whitespace and a plausible sentence start
    if (endIdx >= lineText.length || !/\s/.test(lineText[endIdx])) continue;

    let nextCharIdx = endIdx;
    while (nextCharIdx < lineText.length && /\s/.test(lineText[nextCharIdx])) {
      nextCharIdx++;
    }
    if (nextCharIdx >= lineText.length || !/[A-Z0-9"'(\[]/.test(lineText[nextCharIdx])) {
      continue;
    }

    // Skip abbreviations and single-letter initials like "J. Smith"
    if (ch === '.') {
      const prefix = lineText.slice(start, i + 1);
      const lastWordMatch = prefix.match(/(\S+)$/);
      const lastWord = lastWordMatch ? lastWordMatch[1].toLowerCase() : '';
      if (ABBREVIATIONS.has(lastWord) || /^[A-Za-z]\.$/.test(lastWord)) {
        continue;
      }
    }

    const segment = lineText.slice(start, endIdx).trim();
    if (segment) segments.push(segment);
    start = nextCharIdx;
    i = nextCharIdx - 1;
  }

  const tail = lineText.slice(start).trim();
  if (tail) segments.push(tail);
  return segments;
}

/**
 * Breaks a long text block into speakable chunks so TTS starts immediately
 * while subsequent chunks synthesize in the background, without chopping normal sentences.
 */
function splitIntoSpeechChunks(text: string, maxChars: number = 240): string[] {
  const normalized = text.replace(/\s+/g, ' ').trim();
  if (!normalized) return [];
  if (normalized.length <= maxChars) return [normalized];

  const chunks: string[] = [];
  let remaining = normalized;

  while (remaining.length > maxChars) {
    const window = remaining.slice(0, maxChars + 1);

    // Prefer sentence boundaries, then clause boundaries, then word boundaries
    let splitAt = -1;
    for (const pattern of [/[.!?]["')\]]?\s+(?=[^\s])/g, /[;:—–,]\s+(?=[^\s])/g]) {
      let match: RegExpExecArray | null;
      while ((match = pattern.exec(window)) !== null) {
        if (match.index >= 40) {
          splitAt = match.index + match[0].trimEnd().length;
        }
      }
      if (splitAt !== -1) break;
    }

    if (splitAt === -1) {
      splitAt = window.lastIndexOf(' ');
      if (splitAt < 20) splitAt = maxChars;
    }

    const piece = remaining.slice(0, splitAt).trim();
    if (piece) chunks.push(piece);
    remaining = remaining.slice(splitAt).trim();
  }

  if (remaining) {
    // Avoid awkward tiny 1-2 word trailing chunks
    if (chunks.length > 0 && remaining.length < 25 && chunks[chunks.length - 1].length + remaining.length + 1 <= maxChars + 40) {
      chunks[chunks.length - 1] = `${chunks[chunks.length - 1]} ${remaining}`;
    } else {
      chunks.push(remaining);
    }
  }

  return chunks;
}

function setCachedAudioUrl(key: string, url: string) {
  if (audioCache.value.has(key)) {
    audioCache.value.delete(key);
  } else if (audioCache.value.size >= MAX_AUDIO_CACHE_ENTRIES) {
    const oldestKey = audioCache.value.keys().next().value;
    if (oldestKey !== undefined) {
      const oldestUrl = audioCache.value.get(oldestKey);
      if (oldestUrl) URL.revokeObjectURL(oldestUrl);
      audioCache.value.delete(oldestKey);
    }
  }
  audioCache.value.set(key, url);
}

function clearAudioCache() {
  audioCache.value.forEach((url) => URL.revokeObjectURL(url));
  audioCache.value.clear();
}

function normalizeMarginSignature(text: string): string {
  return text
    .toLowerCase()
    .replace(/\d+/g, '#')
    .replace(/\s+/g, ' ')
    .trim();
}

function isStandalonePageNumber(text: string): boolean {
  return /^(?:[-–—]\s*)?(?:\d{1,4}|[ivxlcdm]{1,7}|page\s+\d+(?:\s*(?:of|\/)\s*\d+)?|\d+\s*\/\s*\d+)(?:\s*[-–—])?$/i.test(
    text.trim()
  );
}

function isMarginBoilerplate(text: string): boolean {
  return /^(?:arxiv:|preprint\b|submitted\s+to\b|under\s+review\b|published\s+(?:as|in)\b|copyright\b|©|doi:|https?:\/\/|proceedings\s+of\b|authorized\s+licensed\s+use|downloaded\s+on\b)/i.test(
    text.trim()
  );
}

function getMedianBodyHeight(elements: PdfPageData['elements']): number {
  const heights = elements
    .map((e) => e.height)
    .filter((h) => h > 0)
    .sort((a, b) => a - b);
  if (heights.length === 0) return 10;
  return heights[Math.floor(heights.length / 2)];
}

export function usePdfReader() {

  const documentName = computed<string>(() => {
    if (!pdfPath.value) return '';
    const parts = pdfPath.value.split(/[/\\]/);
    return parts[parts.length - 1] || 'Document';
  });

  const totalPages = computed<number>(() => pdfPages.value.length);

  // Detect running headers/footers that repeat across multiple pages in top/bottom 12% margins
  const repeatedMarginSignatures = computed<Set<string>>(() => {
    const counts = new Map<string, number>();
    if (pdfPages.value.length < 2) return new Set();

    for (const page of pdfPages.value) {
      const seenOnPage = new Set<string>();
      const topLimit = page.height * 0.12;
      const bottomLimit = page.height * 0.88;

      for (const el of page.elements) {
        const text = el.text.trim();
        if (!text) continue;
        const inMargin = el.y < topLimit || el.y + el.height > bottomLimit;
        if (!inMargin) continue;

        const sig = normalizeMarginSignature(text);
        if (sig.length >= 2) {
          seenOnPage.add(sig);
        }
      }

      seenOnPage.forEach((sig) => {
        counts.set(sig, (counts.get(sig) ?? 0) + 1);
      });
    }

    const repeated = new Set<string>();
    counts.forEach((count, sig) => {
      if (count >= 2) repeated.add(sig);
    });
    return repeated;
  });

  const isHeaderOrFooterElement = (
    page: PdfPageData,
    index: number,
    medianHeight: number
  ): boolean => {
    const el = page.elements[index];
    const text = el.text.trim();
    if (!text) return true;

    const inTopZone = el.y < page.height * 0.11;
    const inBottomZone = el.y + el.height > page.height * 0.89 || el.y > page.height * 0.88;
    if (!inTopZone && !inBottomZone) return false;

    if (isStandalonePageNumber(text)) return true;
    if (repeatedMarginSignatures.value.has(normalizeMarginSignature(text))) return true;
    if (isMarginBoilerplate(text)) return true;

    // Catch isolated small header/footer lines in the extreme margin even on single-page PDFs
    if (el.height <= medianHeight * 1.08 && text.length <= 120) {
      if (el.y < page.height * 0.075) {
        const nextEl = page.elements[index + 1];
        const gapBelow = nextEl ? nextEl.y - (el.y + el.height) : page.height;
        if (gapBelow > el.height * 1.5 && !/[,;\-–]$/.test(text)) {
          return true;
        }
      }
      if (el.y > page.height * 0.915) {
        const prevEl = index > 0 ? page.elements[index - 1] : null;
        const gapAbove = prevEl ? el.y - (prevEl.y + prevEl.height) : page.height;
        if (gapAbove > (prevEl?.height ?? el.height) * 1.5 && !/^[a-z]/.test(text)) {
          return true;
        }
      }
    }

    return false;
  };

  const buildPageSentences = (page: PdfPageData): SentenceBlock[] => {
    const hasLayoutTags = page.elements.some((e) => e.layout_label != null);
    const medianHeight = getMedianBodyHeight(page.elements);
    const sentences: SentenceBlock[] = [];
    let currentText = "";
    let currentIndices: number[] = [];
    let lastIncludedIndex = -1;
    let nextStartsParagraph = true;

    const flushSentence = () => {
      const raw = currentText.trim();
      if (raw) {
        sentences.push({
          text: raw,
          cleanedText: cleanAcademicText(raw),
          elementIndices: [...currentIndices],
          startsParagraph: nextStartsParagraph
        });
        nextStartsParagraph = false;
      }
      currentText = "";
      currentIndices = [];
    };

    for (let i = 0; i < page.elements.length; i++) {
      const el = page.elements[i];
      const text = el.text.trim();
      if (!text) continue;

      if (skipHeadersFooters.value) {
        if (el.is_filtered) continue;
        // Fallback heuristic only when ONNX layout tags are absent
        if (!hasLayoutTags && isHeaderOrFooterElement(page, i, medianHeight)) {
          continue;
        }
      }

      // Check for paragraph/column gaps or heading transitions
      if (lastIncludedIndex >= 0) {
        const prevEl = page.elements[lastIncludedIndex];
        const gap = el.y - (prevEl.y + prevEl.height);
        const labelChanged =
          (prevEl.layout_label === 'doc_title' || prevEl.layout_label === 'paragraph_title') &&
          prevEl.layout_label !== el.layout_label;

        if (labelChanged || gap > prevEl.height * 1.5) {
          flushSentence();
          nextStartsParagraph = true;
        }
      }

      lastIncludedIndex = i;
      const segments = splitLineIntoSegments(text);
      for (let segIdx = 0; segIdx < segments.length; segIdx++) {
        const segment = segments[segIdx];
        const endsWithWrapHyphen = /[\p{L}][-\u2010\u00AD]$/u.test(currentText);
        const startsWithLower = /^[\p{Ll}]/u.test(segment);

        if (segIdx === 0 && endsWithWrapHyphen && startsWithLower) {
          currentText = currentText.slice(0, -1) + segment;
        } else {
          currentText += (currentText ? " " : "") + segment;
        }

        if (currentIndices[currentIndices.length - 1] !== i) {
          currentIndices.push(i);
        }

        if (/[.!?]["')\]]?$/.test(segment)) {
          flushSentence();
        }
      }
    }

    flushSentence();

    return sentences;
  };

  const pageSentences = computed<SentenceBlock[]>(() => {
    const page = pdfPages.value[currentPageIndex.value];
    if (!page) return [];
    return buildPageSentences(page);
  });

  const currentSentence = computed<SentenceBlock | null>(() => {
    if (currentSentenceIndex.value >= 0 && currentSentenceIndex.value < pageSentences.value.length) {
      return pageSentences.value[currentSentenceIndex.value];
    }
    return null;
  });

  const getTextForSentence = (pIndex: number, sIndex: number): string | null => {
    if (pIndex !== currentPageIndex.value) return null;
    const sentence = pageSentences.value[sIndex];
    if (!sentence) return null;

    if (readingMode.value === 'clean') {
      return sentence.cleanedText || cleanAcademicText(sentence.text);
    }
    return sentence.text;
  };

  const getAudioForChunk = (
    pIndex: number,
    sIndex: number,
    chunkIndex: number,
    chunkText: string
  ): Promise<string | null> => {
    const cacheKey = `${pIndex}-s${sIndex}-c${chunkIndex}-${selectedVoice.value}-${readingMode.value}`;
    const cached = audioCache.value.get(cacheKey);
    if (cached) return Promise.resolve(cached);

    const existing = inFlightAudio.get(cacheKey);
    if (existing) return existing;

    const promise = (async () => {
      try {
        const audioResponse = await invoke<ArrayBuffer>('generate_audio', {
          text: chunkText,
          voiceName: selectedVoice.value
        });

        const blob = new Blob([audioResponse], { type: 'audio/wav' });
        const url = URL.createObjectURL(blob);
        setCachedAudioUrl(cacheKey, url);
        return url;
      } catch (error) {
        console.error("Piper TTS synthesis failed:", error);
        return null;
      } finally {
        inFlightAudio.delete(cacheKey);
      }
    })();

    inFlightAudio.set(cacheKey, promise);
    return promise;
  };

  const prefetchNextTarget = (pIndex: number, sIndex: number, chunkIndex: number, chunks: string[]) => {
    if (chunkIndex + 1 < chunks.length) {
      void getAudioForChunk(pIndex, sIndex, chunkIndex + 1, chunks[chunkIndex + 1]);
      return;
    }

    if (sIndex + 1 < pageSentences.value.length) {
      const nextText = getTextForSentence(pIndex, sIndex + 1);
      if (!nextText) return;
      const nextChunks = splitIntoSpeechChunks(nextText);
      if (nextChunks.length > 0) {
        void getAudioForChunk(pIndex, sIndex + 1, 0, nextChunks[0]);
      }
    }
  };

  const attachAudioLifecycle = (
    audio: HTMLAudioElement,
    chunkIndex: number,
    chunks: string[],
    sessionId: number,
    onAdvanceChunk: (nextChunkIdx: number) => void
  ) => {
    audio.onended = () => {
      if (!isPlaying.value || sessionId !== playbackSessionId) return;

      if (chunkIndex + 1 < chunks.length) {
        onAdvanceChunk(chunkIndex + 1);
      } else {
        pausedPlayback = null;
        if (currentAudio === audio) currentAudio = null;
        advanceAfterSentence();
      }
    };

    audio.onerror = (e) => {
      console.error("Audio playback error:", e);
      if (sessionId === playbackSessionId) {
        pausedPlayback = null;
        isPlaying.value = false;
      }
    };
  };

  const playSentenceFromChunk = async (
    pIndex: number,
    sIndex: number,
    chunks: string[],
    startChunkIndex: number,
    sessionId: number
  ) => {
    const playChunk = async (chunkIndex: number) => {
      if (!isPlaying.value || sessionId !== playbackSessionId) return;

      const cacheKey = `${pIndex}-s${sIndex}-c${chunkIndex}-${selectedVoice.value}-${readingMode.value}`;
      let audioUrl: string | null = audioCache.value.get(cacheKey) ?? null;

      if (!audioUrl) {
        isGeneratingAudio.value = true;
        try {
          audioUrl = await getAudioForChunk(pIndex, sIndex, chunkIndex, chunks[chunkIndex]);
        } finally {
          if (sessionId === playbackSessionId) {
            isGeneratingAudio.value = false;
          }
        }
      }

      if (!isPlaying.value || sessionId !== playbackSessionId) return;

      if (!audioUrl) {
        pausedPlayback = null;
        isPlaying.value = false;
        return;
      }

      if (currentAudio) {
        currentAudio.pause();
        currentAudio = null;
      }

      const audio = new Audio(audioUrl);
      audio.playbackRate = speechSpeed.value;
      currentAudio = audio;
      pausedPlayback = {
        pageIndex: pIndex,
        sentenceIndex: sIndex,
        chunkIndex,
        chunks,
        voice: selectedVoice.value,
        readingMode: readingMode.value
      };

      // Pre-synthesize the next chunk (or next sentence) in the background
      prefetchNextTarget(pIndex, sIndex, chunkIndex, chunks);

      attachAudioLifecycle(audio, chunkIndex, chunks, sessionId, (nextIdx) => {
        void playChunk(nextIdx);
      });

      try {
        await audio.play();
      } catch (err) {
        console.error("Audio playback failed to start:", err);
        if (sessionId === playbackSessionId) {
          isPlaying.value = false;
        }
      }
    };

    await playChunk(startChunkIndex);
  };

  const readCurrentSentence = async () => {
    if (!isPlaying.value) return;

    const sessionId = ++playbackSessionId;
    const pIndex = currentPageIndex.value;
    const sIndex = currentSentenceIndex.value;
    const textToSpeak = getTextForSentence(pIndex, sIndex);

    if (!textToSpeak) {
      pausedPlayback = null;
      isPlaying.value = false;
      return;
    }

    const chunks = splitIntoSpeechChunks(textToSpeak);
    if (chunks.length === 0) {
      nextSentence();
      return;
    }

    await playSentenceFromChunk(pIndex, sIndex, chunks, 0, sessionId);
  };

  const setSpeechSpeed = (speed: number) => {
    speechSpeed.value = speed;
    if (currentAudio) {
      currentAudio.playbackRate = speed;
    }
  };

  const setZoomLevel = (zoom: number) => {
    zoomLevel.value = Math.min(2.5, Math.max(0.6, zoom));
  };

  const zoomIn = () => setZoomLevel(zoomLevel.value + 0.15);
  const zoomOut = () => setZoomLevel(zoomLevel.value - 0.15);
  const resetZoom = () => setZoomLevel(1.0);

  const setTheme = (newTheme: AppTheme) => {
    theme.value = newTheme;
    document.documentElement.setAttribute('data-theme', newTheme);
  };

  const setReadingMode = (mode: ReadingMode) => {
    readingMode.value = mode;
    pausedPlayback = null;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio = null;
    }
    // Clear cache so changes take effect immediately
    clearAudioCache();
  };

  watch(selectedVoice, () => {
    pausedPlayback = null;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio = null;
    }
    if (isPlaying.value) {
      void readCurrentSentence();
    }
  });

  const toggleSkipHeadersFooters = () => {
    stopReading();
    skipHeadersFooters.value = !skipHeadersFooters.value;
    currentSentenceIndex.value = -1;
  };

  const jumpToSentence = (sIndex: number, autoPlay: boolean = true) => {
    if (sIndex < 0 || sIndex >= pageSentences.value.length) return;
    playbackSessionId++;
    pausedPlayback = null;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    currentSentenceIndex.value = sIndex;
    if (autoPlay || isPlaying.value) {
      isPlaying.value = true;
      readCurrentSentence();
    }
  };

  const jumpToPage = (targetPage: number) => {
    const pageIndex = targetPage - 1;
    if (pageIndex >= 0 && pageIndex < totalPages.value) {
      stopReading();
      currentPageIndex.value = pageIndex;
      currentSentenceIndex.value = -1;
    }
  };

  const openFile = async () => {
    try {
      const selectedFile = await open({
        multiple: false,
        directory: false,
        filters: [{
          name: 'PDF Documents',
          extensions: ['pdf']
        }]
      });

      if (selectedFile) {
        await loadDocumentPath(selectedFile as string);
      }
    } catch (error) {
      console.error("Failed to open file:", error);
    }
  };

  const loadDocumentPath = async (filePath: string) => {
    stopReading();
    pdfPath.value = filePath;
    isLoading.value = true;
    try {
      pdfUrl.value = convertFileSrc(pdfPath.value);
      pdfPages.value = await invoke<PdfPageData[]>('parse_pdf', { path: pdfPath.value });
      currentSentenceIndex.value = -1;
      currentPageIndex.value = 0;
    } catch (error) {
      console.error("Error loading PDF document:", error);
      alert(`Could not load document: ${error}`);
    } finally {
      isLoading.value = false;
    }
  };

  const advanceAfterSentence = () => {
    if (currentSentenceIndex.value < pageSentences.value.length - 1) {
      currentSentenceIndex.value++;
      if (isPlaying.value) readCurrentSentence();
    } else if (currentPageIndex.value < pdfPages.value.length - 1) {
      currentPageIndex.value++;
      currentSentenceIndex.value = 0;
      if (isPlaying.value) readCurrentSentence();
    } else {
      stopReading();
    }
  };

  const nextSentence = () => {
    playbackSessionId++;
    pausedPlayback = null;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    advanceAfterSentence();
  };

  const prevSentence = () => {
    playbackSessionId++;
    pausedPlayback = null;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    if (currentSentenceIndex.value > 0) {
      currentSentenceIndex.value--;
      if (isPlaying.value) readCurrentSentence();
    } else if (currentPageIndex.value > 0) {
      currentPageIndex.value--;
      currentSentenceIndex.value = 0;
      if (isPlaying.value) readCurrentSentence();
    }
  };

  const prevPage = () => {
    if (currentPageIndex.value > 0) {
      stopReading();
      currentPageIndex.value--;
      currentSentenceIndex.value = -1;
    }
  };

  const nextPage = () => {
    if (currentPageIndex.value < totalPages.value - 1) {
      stopReading();
      currentPageIndex.value++;
      currentSentenceIndex.value = -1;
    }
  };

  const pauseReading = () => {
    playbackSessionId++;
    isPlaying.value = false;
    isGeneratingAudio.value = false;
    if (currentAudio && !currentAudio.ended) {
      currentAudio.pause();
    } else {
      pausedPlayback = null;
    }
  };

  const resumeReading = async () => {
    if (
      currentAudio &&
      pausedPlayback &&
      pausedPlayback.pageIndex === currentPageIndex.value &&
      pausedPlayback.sentenceIndex === currentSentenceIndex.value &&
      pausedPlayback.voice === selectedVoice.value &&
      pausedPlayback.readingMode === readingMode.value &&
      !currentAudio.ended
    ) {
      const resumed = pausedPlayback;
      const audio = currentAudio;
      const sessionId = ++playbackSessionId;

      isPlaying.value = true;
      audio.playbackRate = speechSpeed.value;
      attachAudioLifecycle(
        audio,
        resumed.chunkIndex,
        resumed.chunks,
        sessionId,
        (nextChunkIdx) => {
          void playSentenceFromChunk(
            resumed.pageIndex,
            resumed.sentenceIndex,
            resumed.chunks,
            nextChunkIdx,
            sessionId
          );
        }
      );

      try {
        await audio.play();
        return;
      } catch (err) {
        console.error("Failed to resume paused audio:", err);
        pausedPlayback = null;
      }
    }

    isPlaying.value = true;
    if (currentSentenceIndex.value === -1) currentSentenceIndex.value = 0;
    await readCurrentSentence();
  };

  const toggleReading = () => {
    if (isPlaying.value) {
      pauseReading();
    } else {
      void resumeReading();
    }
  };

  const stopReading = () => {
    playbackSessionId++;
    pausedPlayback = null;
    isPlaying.value = false;
    isGeneratingAudio.value = false;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    clearAudioCache();
  };

  const getSentenceReadText = (sentence: SentenceBlock): string => {
    if (readingMode.value === 'clean') {
      return sentence.cleanedText || cleanAcademicText(sentence.text);
    }
    return sentence.text;
  };

  const buildFullDocumentText = (asMarkdown: boolean): string => {
    const baseTitle = documentName.value.replace(/\.pdf$/i, '') || 'Document';
    const sections: string[] = [];

    if (asMarkdown) {
      sections.push(`# ${baseTitle}`);
    }

    for (const page of pdfPages.value) {
      const sentences = buildPageSentences(page);
      if (sentences.length === 0) continue;

      const paragraphs: string[] = [];
      let currentParagraph: string[] = [];

      for (const sentence of sentences) {
        const text = getSentenceReadText(sentence).trim();
        if (!text) continue;

        if (sentence.startsParagraph && currentParagraph.length > 0) {
          paragraphs.push(currentParagraph.join(' '));
          currentParagraph = [];
        }
        currentParagraph.push(text);
      }

      if (currentParagraph.length > 0) {
        paragraphs.push(currentParagraph.join(' '));
      }

      if (paragraphs.length > 0) {
        if (asMarkdown) {
          sections.push(`## Page ${page.page_number}\n\n${paragraphs.join('\n\n')}`);
        } else {
          sections.push(paragraphs.join('\n\n'));
        }
      }
    }

    return sections.join('\n\n');
  };

  const exportFullText = async () => {
    if (pdfPages.value.length === 0) return;
    const baseName = documentName.value.replace(/\.pdf$/i, '') || 'document';

    try {
      const targetPath = await save({
        defaultPath: `${baseName}_read_text.txt`,
        filters: [
          { name: 'Plain Text', extensions: ['txt'] },
          { name: 'Markdown', extensions: ['md'] }
        ]
      });
      if (!targetPath) return;

      const asMarkdown = targetPath.toLowerCase().endsWith('.md');
      const content = buildFullDocumentText(asMarkdown);
      await invoke('save_text_file', { path: targetPath, content });
    } catch (error) {
      console.error('Failed to export text:', error);
      alert(`Could not save text: ${error}`);
    }
  };

  const exportFullAudio = async () => {
    if (pdfPages.value.length === 0 || isExportingAudio.value) return;
    const baseName = documentName.value.replace(/\.pdf$/i, '') || 'document';

    const chunks: string[] = [];
    for (const page of pdfPages.value) {
      for (const sentence of buildPageSentences(page)) {
        const text = getSentenceReadText(sentence);
        chunks.push(...splitIntoSpeechChunks(text));
      }
    }

    if (chunks.length === 0) {
      alert('No readable text found in this PDF.');
      return;
    }

    let unlisten: (() => void) | null = null;
    try {
      const targetPath = await save({
        defaultPath: `${baseName}_audio.wav`,
        filters: [{ name: 'WAV Audio', extensions: ['wav'] }]
      });
      if (!targetPath) return;

      isExportingAudio.value = true;
      exportProgress.value = { current: 0, total: chunks.length };

      unlisten = await listen<{ current: number; total: number }>(
        'audio-export-progress',
        (event) => {
          exportProgress.value = event.payload;
        }
      );

      await invoke('export_audio_file', {
        path: targetPath,
        chunks,
        voiceName: selectedVoice.value
      });
    } catch (error) {
      const msg = String(error);
      if (!msg.includes('cancelled')) {
        console.error('Failed to export audio:', error);
        alert(`Could not export audio: ${msg}`);
      }
    } finally {
      if (unlisten) unlisten();
      isExportingAudio.value = false;
    }
  };

  const cancelAudioExport = async () => {
    try {
      await invoke('cancel_audio_export');
    } catch (error) {
      console.error('Failed to cancel export:', error);
    }
  };

  return {
    pdfPath,
    pdfUrl,
    pdfPages,
    currentPageIndex,
    totalPages,
    documentName,
    isLoading,
    isPlaying,
    isGeneratingAudio,
    selectedVoice,
    speechSpeed,
    zoomLevel,
    theme,
    readingMode,
    skipHeadersFooters,
    currentSentenceIndex,
    currentSentence,
    pageSentences,
    isSidebarOpen,
    activeSidebarTab,
    isExportingAudio,
    exportProgress,
    openFile,
    loadDocumentPath,
    toggleReading,
    stopReading,
    prevPage,
    nextPage,
    prevSentence,
    nextSentence,
    jumpToSentence,
    jumpToPage,
    setSpeechSpeed,
    setZoomLevel,
    zoomIn,
    zoomOut,
    resetZoom,
    setTheme,
    setReadingMode,
    toggleSkipHeadersFooters,
    exportFullText,
    exportFullAudio,
    cancelAudioExport
  };
}
