import { ref, computed } from 'vue';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

import type { PdfPageData, SentenceBlock, ReadingMode, AppTheme, ExplanationResult } from '../types/pdf';

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
const isSidebarOpen = ref<boolean>(false);
const activeSidebarTab = ref<'transcript' | 'explain' | 'pages'>('transcript');
const isExplaining = ref<boolean>(false);
const currentExplanation = ref<ExplanationResult | null>(null);

let currentAudio: HTMLAudioElement | null = null;
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
    // Join hyphenated words split across lines: "infor- \n mation" -> "information"
    .replace(/(\b\w+)-\s+(\w+\b)/g, '$1$2')
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

function clearAudioCache() {
  audioCache.value.forEach((url) => URL.revokeObjectURL(url));
  audioCache.value.clear();
}

export function usePdfReader() {

  const documentName = computed<string>(() => {
    if (!pdfPath.value) return '';
    const parts = pdfPath.value.split(/[/\\]/);
    return parts[parts.length - 1] || 'Document';
  });

  const totalPages = computed<number>(() => pdfPages.value.length);

  const pageSentences = computed<SentenceBlock[]>(() => {
    const page = pdfPages.value[currentPageIndex.value];
    if (!page) return [];

    const sentences: SentenceBlock[] = [];
    let currentText = "";
    let currentIndices: number[] = [];

    const flushSentence = () => {
      const raw = currentText.trim();
      if (raw) {
        sentences.push({
          text: raw,
          cleanedText: cleanAcademicText(raw),
          elementIndices: [...currentIndices]
        });
      }
      currentText = "";
      currentIndices = [];
    };

    for (let i = 0; i < page.elements.length; i++) {
      const el = page.elements[i];
      const text = el.text.trim();
      if (!text) continue;

      // Check if there is a massive vertical gap (a new paragraph)
      if (i > 0 && currentIndices.length > 0) {
        const prevEl = page.elements[i - 1];
        const gap = el.y - (prevEl.y + prevEl.height);

        // If gap is larger than 1.5x line height, break sentence
        if (gap > prevEl.height * 1.5) {
          flushSentence();
        }
      }

      const segments = splitLineIntoSegments(text);
      for (const segment of segments) {
        currentText += (currentText ? " " : "") + segment;
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

    if (readingMode.value === 'clean' || readingMode.value === 'explanatory') {
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
        audioCache.value.set(cacheKey, url);
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

  const readCurrentSentence = async () => {
    if (!isPlaying.value) return;

    const sessionId = ++playbackSessionId;
    const pIndex = currentPageIndex.value;
    const sIndex = currentSentenceIndex.value;
    const textToSpeak = getTextForSentence(pIndex, sIndex);

    if (!textToSpeak) {
      isPlaying.value = false;
      return;
    }

    const chunks = splitIntoSpeechChunks(textToSpeak);
    if (chunks.length === 0) {
      nextSentence();
      return;
    }

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

      // Pre-synthesize the next chunk (or next sentence) in the background
      prefetchNextTarget(pIndex, sIndex, chunkIndex, chunks);

      audio.onended = () => {
        URL.revokeObjectURL(audioUrl);
        audioCache.value.delete(cacheKey);

        if (!isPlaying.value || sessionId !== playbackSessionId) return;

        if (chunkIndex + 1 < chunks.length) {
          void playChunk(chunkIndex + 1);
        } else {
          advanceAfterSentence();
        }
      };

      audio.onerror = (e) => {
        console.error("Audio playback error:", e);
        if (sessionId === playbackSessionId) {
          isPlaying.value = false;
        }
      };

      try {
        await audio.play();
      } catch (err) {
        console.error("Audio playback failed to start:", err);
        if (sessionId === playbackSessionId) {
          isPlaying.value = false;
        }
      }
    };

    await playChunk(0);
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
    // Clear cache so changes take effect immediately
    clearAudioCache();
  };

  const jumpToSentence = (sIndex: number, autoPlay: boolean = true) => {
    if (sIndex < 0 || sIndex >= pageSentences.value.length) return;
    playbackSessionId++;
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
      currentExplanation.value = null;
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
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    advanceAfterSentence();
  };

  const prevSentence = () => {
    playbackSessionId++;
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

  const toggleReading = () => {
    if (isPlaying.value) {
      stopReading();
    } else {
      isPlaying.value = true;
      if (currentSentenceIndex.value === -1) currentSentenceIndex.value = 0;
      readCurrentSentence();
    }
  };

  const stopReading = () => {
    playbackSessionId++;
    isPlaying.value = false;
    isGeneratingAudio.value = false;
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.currentTime = 0;
      currentAudio = null;
    }
    clearAudioCache();
  };

  /**
   * Explanatory Engine for Doctoral Theses:
   * Translates the active sentence or paragraph into intuitive concepts.
   */
  const requestExplanation = async () => {
    const activeSentence = currentSentence.value;
    if (!activeSentence) return;

    isExplaining.value = true;
    try {
      // Collect surrounding paragraph context (current sentence +/- 1)
      const sIndex = currentSentenceIndex.value;
      const prev = sIndex > 0 ? pageSentences.value[sIndex - 1].text : '';
      const curr = activeSentence.text;
      const next = sIndex < pageSentences.value.length - 1 ? pageSentences.value[sIndex + 1].text : '';
      const context = [prev, curr, next].filter(Boolean).join(" ");

      // Rule-based academic extractor & synthesis
      const isMethodological = /method|algorithm|equation|theorem|model|framework|sample|dataset/i.test(context);
      const isResults = /increase|decrease|significant|p-value|accuracy|correlate|observed|demonstrate/i.test(context);

      let summary = "";
      let intuition = "";
      const terms: { term: string; definition: string }[] = [];

      if (isMethodological) {
        summary = "Methodology & Architecture: The author describes how the model or experiment was designed and formalized.";
        intuition = "Think of this as setting up the rules and measuring instruments of the research before testing the hypotheses.";
      } else if (isResults) {
        summary = "Empirical Finding: Discusses observed quantitative or qualitative metrics and behavioral trends.";
        intuition = "The research highlights concrete evidence answering the central dissertation research question.";
      } else {
        summary = "Conceptual Argument: Establishes theoretical grounding or synthesizes prior scientific literature.";
        intuition = "The dissertation connects this claim with the broader consensus and identified gaps in the field.";
      }

      // Detect potential jargon acronyms (2-5 capital letters)
      const acronymMatches = context.match(/\b[A-Z]{2,6}\b/g) || [];
      const uniqueAcronyms = [...new Set(acronymMatches)];
      uniqueAcronyms.slice(0, 3).forEach(term => {
        terms.push({
          term,
          definition: `Core domain acronym used in this thesis context to represent a specific model, metric, or framework.`
        });
      });

      currentExplanation.value = {
        summary,
        intuition,
        terms
      };
      isSidebarOpen.value = true;
      activeSidebarTab.value = 'explain';
    } finally {
      isExplaining.value = false;
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
    currentSentenceIndex,
    currentSentence,
    pageSentences,
    isSidebarOpen,
    activeSidebarTab,
    isExplaining,
    currentExplanation,
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
    requestExplanation
  };
}
