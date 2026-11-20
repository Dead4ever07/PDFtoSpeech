<template>
  <div
    class="viewer-container"
    @dragover.prevent="handleDragOver"
    @dragleave.prevent="handleDragLeave"
    @drop.prevent="handleDrop"
    :class="{ 'drag-active': isDragging }"
  >
    <!-- 1. Loading Overlay -->
    <div v-if="isLoading" class="loading-overlay">
      <div class="loading-spinner"></div>
      <p class="loading-text">Parsing PDF & Generating Academic Structure...</p>
    </div>

    <!-- 2. Active PDF Viewer -->
    <div
      v-else-if="pdfUrl && activePage"
      class="page-wrapper"
      :style="{
        width: `${800 * zoomLevel}px`,
        aspectRatio: `${activePage.width} / ${activePage.height}`
      }"
    >
      <!-- Rendered PDF Page -->
      <VuePdfEmbed
        :source="pdfUrl"
        :page="activePage.page_number"
        :text-layer="false"
        :annotation-layer="false"
        class="pdf-render-layer"
      />

      <!-- Interactive Highlight & Click-to-Read Layer -->
      <div class="highlight-layer">
        <div
          v-for="(sentence, sIdx) in pageSentences"
          :key="sIdx"
          class="sentence-group"
          :class="{ 'is-active-sentence': currentSentenceIndex === sIdx }"
          @click="jumpToSentence(sIdx)"
          :title="`Sentence ${sIdx + 1}: Click to read aloud`"
        >
          <template v-for="lineIdx in sentence.elementIndices" :key="lineIdx">
            <div
              v-if="activePage.elements[lineIdx]"
              class="highlight-box"
              :style="{
                left: `${(activePage.elements[lineIdx].x / activePage.width) * 100}%`,
                top: `${(activePage.elements[lineIdx].y / activePage.height) * 100}%`,
                width: `${(activePage.elements[lineIdx].width / activePage.width) * 100}%`,
                height: `${(activePage.elements[lineIdx].height / activePage.height) * 100}%`
              }"
            ></div>
          </template>
        </div>
      </div>
    </div>

    <!-- 3. Welcome / Empty Dropzone State -->
    <div v-else class="welcome-container">
      <div class="welcome-card">
        <div class="hero-icon">📖</div>
        <h2 class="welcome-title">PDFtoSpeech Academic Reader</h2>
        <p class="welcome-subtitle">
          Engineered for reading and understanding doctoral dissertations, research papers, and technical books.
        </p>

        <div class="drop-target" @click="openFile">
          <span class="upload-icon">📂</span>
          <p class="drop-text">Click to Open or Drop a PDF File Here</p>
          <span class="drop-subtext">Supports single-column and multi-column academic papers</span>
        </div>

        <div class="shortcuts-guide">
          <h4 class="guide-title">⚡ Keyboard Shortcuts</h4>
          <div class="shortcuts-grid">
            <div class="shortcut-item">
              <kbd>Space</kbd>
              <span>Play / Pause</span>
            </div>
            <div class="shortcut-item">
              <kbd>←</kbd> <kbd>→</kbd>
              <span>Previous / Next Page</span>
            </div>
            <div class="shortcut-item">
              <kbd>↑</kbd> <kbd>↓</kbd>
              <span>Previous / Next Sentence</span>
            </div>
            <div class="shortcut-item">
              <kbd>Click</kbd>
              <span>Jump & Read Any Sentence</span>
            </div>
            <div class="shortcut-item">
              <kbd>Esc</kbd>
              <span>Stop Reading</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue';
import VuePdfEmbed from 'vue-pdf-embed';
import { usePdfReader } from '../composables/usePdfReader';
import type { PdfPageData } from '../types/pdf';

const {
  pdfUrl,
  pdfPages,
  currentPageIndex,
  currentSentenceIndex,
  pageSentences,
  zoomLevel,
  isLoading,
  openFile,
  loadDocumentPath,
  jumpToSentence
} = usePdfReader();

const isDragging = ref(false);

const activePage = computed<PdfPageData | undefined>(() => {
  return pdfPages.value[currentPageIndex.value];
});

// Auto-scroll the active sentence into view
watch(currentSentenceIndex, (newVal) => {
  if (newVal >= 0) {
    nextTick(() => {
      const activeEl = document.querySelector('.sentence-group.is-active-sentence');
      if (activeEl) {
        activeEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    });
  }
});

// Drag and drop PDF files
const handleDragOver = (e: DragEvent) => {
  if (e.dataTransfer?.types.includes('Files')) {
    isDragging.value = true;
  }
};

const handleDragLeave = () => {
  isDragging.value = false;
};

const handleDrop = async (e: DragEvent) => {
  isDragging.value = false;
  if (!e.dataTransfer || !e.dataTransfer.files.length) return;

  const file = e.dataTransfer.files[0];
  if (file && file.name.endsWith('.pdf')) {
    // In Tauri webview, file may have a path attribute
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const filePath = (file as any).path || '';
    if (filePath) {
      await loadDocumentPath(filePath);
    } else {
      await openFile();
    }
  }
};
</script>

<style scoped>
.viewer-container {
  flex: 1;
  overflow-y: auto;
  overflow-x: auto;
  padding: 2.5rem 1rem;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  background-color: var(--viewer-bg, #f1f5f9);
  position: relative;
  transition: background-color 0.2s;
}

.viewer-container.drag-active {
  background-color: var(--accent-light, #eff6ff);
  outline: 3px dashed var(--accent-color, #2563eb);
  outline-offset: -12px;
}

/* Page wrapper holding canvas & highlight layer */
.page-wrapper {
  position: relative;
  max-width: 98vw;
  background: white;
  box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.15), 0 8px 10px -6px rgba(0, 0, 0, 0.1);
  border-radius: 4px;
  margin: 0 auto 3rem auto;
  transition: width 0.15s ease;
}

.pdf-render-layer {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
}

:deep(.vue-pdf-embed canvas) {
  width: 100% !important;
  height: 100% !important;
  display: block;
}

/* Highlights */
.highlight-layer {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  z-index: 10;
}

.sentence-group {
  cursor: pointer;
}

.sentence-group:hover .highlight-box {
  background-color: rgba(59, 130, 246, 0.15);
}

.highlight-box {
  position: absolute;
  background-color: transparent;
  border-radius: 3px;
  transition: background-color 0.1s ease, box-shadow 0.1s ease;
}

.sentence-group.is-active-sentence .highlight-box {
  background-color: var(--highlight-bg, rgba(245, 158, 11, 0.38));
  box-shadow: 0 0 4px var(--highlight-glow, rgba(245, 158, 11, 0.65));
}

/* Loading Overlay */
.loading-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(255, 255, 255, 0.85);
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  gap: 16px;
  z-index: 60;
  backdrop-filter: blur(4px);
}

.loading-spinner {
  width: 44px;
  height: 44px;
  border: 4px solid #e2e8f0;
  border-top-color: var(--accent-color, #2563eb);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.loading-text {
  font-weight: 600;
  color: var(--text-primary, #0f172a);
  font-size: 14px;
}

/* Welcome Screen */
.welcome-container {
  display: flex;
  justify-content: center;
  align-items: center;
  width: 100%;
  height: 100%;
  min-height: 480px;
}

.welcome-card {
  max-width: 600px;
  background: var(--card-bg, #ffffff);
  border: 1px solid var(--border-color, #e2e8f0);
  border-radius: 16px;
  padding: 36px 32px;
  text-align: center;
  box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.08), 0 8px 10px -6px rgba(0, 0, 0, 0.04);
}

.hero-icon {
  font-size: 48px;
  margin-bottom: 12px;
}

.welcome-title {
  font-size: 24px;
  font-weight: 800;
  color: var(--text-primary, #0f172a);
  margin: 0 0 8px 0;
  letter-spacing: -0.5px;
}

.welcome-subtitle {
  font-size: 14px;
  line-height: 1.5;
  color: var(--text-secondary, #64748b);
  margin: 0 0 24px 0;
}

.drop-target {
  background: var(--badge-bg, #f8fafc);
  border: 2px dashed var(--border-color, #cbd5e1);
  border-radius: 12px;
  padding: 28px 20px;
  cursor: pointer;
  transition: all 0.15s ease;
  margin-bottom: 24px;
}

.drop-target:hover {
  background: var(--accent-light, #eff6ff);
  border-color: var(--accent-color, #2563eb);
  transform: translateY(-2px);
}

.upload-icon {
  font-size: 32px;
  display: block;
  margin-bottom: 6px;
}

.drop-text {
  font-size: 15px;
  font-weight: 700;
  color: var(--accent-color, #2563eb);
  margin: 0 0 4px 0;
}

.drop-subtext {
  font-size: 12px;
  color: var(--text-muted, #94a3b8);
}

.shortcuts-guide {
  border-top: 1px solid var(--border-color, #e2e8f0);
  padding-top: 20px;
  text-align: left;
}

.guide-title {
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary, #475569);
  margin: 0 0 12px 0;
}

.shortcuts-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.shortcut-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary, #334155);
}

kbd {
  background: var(--badge-bg, #f1f5f9);
  border: 1px solid var(--border-color, #cbd5e1);
  border-radius: 4px;
  padding: 2px 6px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 700;
  color: var(--text-primary, #0f172a);
  box-shadow: 0 1px 1px rgba(0, 0, 0, 0.1);
}
</style>
