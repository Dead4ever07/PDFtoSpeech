<template>
  <div class="toolbar-container" v-if="totalPages > 0">
    <div class="toolbar-content">

      <!-- Playback Controls Cluster -->
      <div class="control-group playback-group">
        <button
          class="nav-btn icon-btn"
          @click="prevSentence"
          :disabled="currentPageIndex === 0 && currentSentenceIndex <= 0"
          title="Previous Sentence (↑)"
        >
          ⏮
        </button>

        <button
          @click="toggleReading"
          class="play-btn"
          :class="{ 'is-playing': isPlaying, 'is-loading': isGeneratingAudio }"
          title="Play / Pause (Space)"
        >
          <span v-if="isGeneratingAudio" class="spinner">⏳</span>
          <span v-else-if="isPlaying">⏸ Pause</span>
          <span v-else>▶ Play</span>
        </button>

        <button
          class="nav-btn icon-btn"
          @click="stopReading"
          :disabled="!isPlaying && currentSentenceIndex === -1"
          title="Stop (Esc)"
        >
          ⏹
        </button>

        <button
          class="nav-btn icon-btn"
          @click="nextSentence"
          :disabled="currentPageIndex === totalPages - 1 && currentSentenceIndex >= pageSentences.length - 1"
          title="Next Sentence (↓)"
        >
          ⏭
        </button>
      </div>

      <div class="divider"></div>

      <!-- Page Navigation Cluster with Direct Jump -->
      <div class="control-group pagination-group">
        <button
          class="nav-btn"
          @click="prevPage"
          :disabled="currentPageIndex === 0"
          title="Previous Page (←)"
        >
          ◀
        </button>

        <div class="page-jump">
          <span>Page</span>
          <input
            type="number"
            class="page-input"
            :value="currentPageIndex + 1"
            min="1"
            :max="totalPages"
            @change="handlePageInput"
            @keydown.enter="handlePageInput"
          />
          <span>of {{ totalPages }}</span>
        </div>

        <button
          class="nav-btn"
          @click="nextPage"
          :disabled="currentPageIndex === totalPages - 1"
          title="Next Page (→)"
        >
          ▶
        </button>
      </div>

      <div class="divider"></div>

      <!-- Reading Mode Selector (Academic Focus) -->
      <div class="control-group mode-group">
        <div class="mode-selector" title="Reading Mode for Doctoral Theses">
          <button
            class="mode-btn"
            :class="{ active: readingMode === 'verbatim' }"
            @click="setReadingMode('verbatim')"
            title="Read exact text verbatim"
          >
            Verbatim
          </button>
          <button
            class="mode-btn"
            :class="{ active: readingMode === 'clean' }"
            @click="setReadingMode('clean')"
            title="Clean Academic: removes citation brackets [1, 2], (Author, 2020), and hyphenation noise"
          >
            Clean Academic
          </button>
        </div>
      </div>

      <div class="divider"></div>

      <!-- Voice and Speed Settings -->
      <div class="control-group audio-settings">
        <!-- Voice selector -->
        <div class="setting-item">
          <span class="setting-label">Voice:</span>
          <select v-model="selectedVoice" class="select-input">
            <optgroup label="Kokoro Neural (Natural)">
              <option value="kokoro_af_heart">Heart (US Natural)</option>
              <option value="kokoro_af_bella">Bella (US Warm)</option>
              <option value="kokoro_am_michael">Michael (US Male)</option>
              <option value="kokoro_am_adam">Adam (US Crisp)</option>
              <option value="kokoro_bf_emma">Emma (UK Natural)</option>
              <option value="kokoro_bm_george">George (UK Male)</option>
            </optgroup>
            <optgroup label="Piper (Fast Fallback)">
              <option value="alba">Alba (UK Fast)</option>
              <option value="joe">Joe (US Fast)</option>
            </optgroup>
          </select>
        </div>

        <!-- Speed selector -->
        <div class="setting-item">
          <span class="setting-label">Speed:</span>
          <select
            :value="speechSpeed"
            @change="handleSpeedChange"
            class="select-input speed-select"
          >
            <option :value="0.8">0.8x</option>
            <option :value="1.0">1.0x (Normal)</option>
            <option :value="1.2">1.2x</option>
            <option :value="1.35">1.35x</option>
            <option :value="1.5">1.5x</option>
            <option :value="1.75">1.75x</option>
            <option :value="2.0">2.0x</option>
          </select>
        </div>
      </div>

      <div class="divider"></div>

      <!-- Zoom Controls -->
      <div class="control-group zoom-group">
        <button class="zoom-btn" @click="zoomOut" title="Zoom Out (-)">−</button>
        <span class="zoom-label" @click="resetZoom" title="Click to reset zoom">{{ Math.round(zoomLevel * 100) }}%</span>
        <button class="zoom-btn" @click="zoomIn" title="Zoom In (+)">+</button>
      </div>

      <div class="divider"></div>

      <!-- Explanatory Action Button -->
      <div class="control-group explain-group">
        <button
          class="explain-btn"
          :class="{ 'is-explaining': isExplaining }"
          @click="requestExplanation"
          :disabled="currentSentenceIndex === -1"
          title="Explain current sentence/concept in intuitive plain English"
        >
          <span class="explain-icon">💡</span>
          <span>Explain</span>
        </button>
      </div>

    </div>

    <!-- Active Sentence Sub-bar / Teleprompter ticker -->
    <div v-if="currentSentence" class="sentence-ticker">
      <span class="ticker-badge">Now Reading:</span>
      <span class="ticker-text">{{ currentSentence.cleanedText || currentSentence.text }}</span>
      <span class="ticker-progress">({{ currentSentenceIndex + 1 }} / {{ pageSentences.length }})</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { usePdfReader } from '../composables/usePdfReader';

const {
  currentPageIndex,
  totalPages,
  isPlaying,
  isGeneratingAudio,
  selectedVoice,
  speechSpeed,
  zoomLevel,
  readingMode,
  currentSentenceIndex,
  currentSentence,
  pageSentences,
  isExplaining,
  toggleReading,
  stopReading,
  prevPage,
  nextPage,
  prevSentence,
  nextSentence,
  jumpToPage,
  setSpeechSpeed,
  zoomIn,
  zoomOut,
  resetZoom,
  setReadingMode,
  requestExplanation
} = usePdfReader();

const handlePageInput = (event: Event) => {
  const target = event.target as HTMLInputElement;
  const page = parseInt(target.value, 10);
  if (!isNaN(page)) {
    jumpToPage(page);
  }
};

const handleSpeedChange = (event: Event) => {
  const target = event.target as HTMLSelectElement;
  setSpeechSpeed(parseFloat(target.value));
};
</script>

<style scoped>
.toolbar-container {
  background: var(--toolbar-bg, #ffffff);
  border-bottom: 1px solid var(--border-color, #e2e8f0);
  box-shadow: 0 4px 12px -2px rgba(0, 0, 0, 0.05);
  z-index: 50;
  display: flex;
  flex-direction: column;
}

.toolbar-content {
  padding: 8px 16px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 12px;
}

.control-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.divider {
  width: 1px;
  height: 24px;
  background-color: var(--border-color, #e2e8f0);
}

/* Playback buttons */
.play-btn {
  background: linear-gradient(135deg, #10b981 0%, #059669 100%);
  color: white;
  font-weight: 700;
  border: none;
  border-radius: 8px;
  padding: 7px 18px;
  font-size: 13px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 6px;
  box-shadow: 0 2px 6px rgba(16, 185, 129, 0.3);
  transition: all 0.15s ease;
  min-width: 96px;
  justify-content: center;
}

.play-btn:hover {
  transform: translateY(-1px);
  box-shadow: 0 4px 10px rgba(16, 185, 129, 0.4);
}

.play-btn.is-playing {
  background: linear-gradient(135deg, #f59e0b 0%, #d97706 100%);
  box-shadow: 0 2px 6px rgba(245, 158, 11, 0.3);
}

.play-btn.is-loading {
  background: linear-gradient(135deg, #6366f1 0%, #4f46e5 100%);
}

.nav-btn {
  background: var(--card-bg, #ffffff);
  border: 1px solid var(--border-color, #cbd5e1);
  color: var(--text-primary, #1e293b);
  border-radius: 6px;
  padding: 6px 10px;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.12s;
  display: flex;
  align-items: center;
  justify-content: center;
}

.nav-btn.icon-btn {
  padding: 6px 8px;
  font-size: 12px;
}

.nav-btn:hover:not(:disabled) {
  border-color: var(--accent-color, #2563eb);
  color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
}

.nav-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Page Jump */
.page-jump {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary, #475569);
}

.page-input {
  width: 48px;
  padding: 4px 6px;
  border-radius: 6px;
  border: 1px solid var(--border-color, #cbd5e1);
  background: var(--card-bg, #ffffff);
  color: var(--text-primary, #0f172a);
  text-align: center;
  font-weight: 700;
  font-size: 12px;
}

.page-input:focus {
  outline: none;
  border-color: var(--accent-color, #2563eb);
  box-shadow: 0 0 0 2px rgba(37, 99, 235, 0.2);
}

/* Mode Selector */
.mode-selector {
  display: flex;
  background: var(--badge-bg, #f1f5f9);
  padding: 3px;
  border-radius: 8px;
  border: 1px solid var(--border-color, #e2e8f0);
}

.mode-btn {
  border: none;
  background: transparent;
  color: var(--text-secondary, #64748b);
  padding: 4px 10px;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}

.mode-btn:hover {
  color: var(--text-primary, #0f172a);
}

.mode-btn.active {
  background: var(--card-bg, #ffffff);
  color: var(--accent-color, #2563eb);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
}

/* Setting Items */
.setting-item {
  display: flex;
  align-items: center;
  gap: 4px;
}

.setting-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted, #64748b);
}

.select-input {
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid var(--border-color, #cbd5e1);
  background: var(--card-bg, #ffffff);
  color: var(--text-primary, #0f172a);
  font-size: 12px;
  cursor: pointer;
  outline: none;
}

.select-input.speed-select {
  width: 90px;
}

.select-input:focus {
  border-color: var(--accent-color, #2563eb);
}

/* Zoom controls */
.zoom-group {
  display: flex;
  align-items: center;
  background: var(--badge-bg, #f1f5f9);
  border-radius: 6px;
  border: 1px solid var(--border-color, #e2e8f0);
  padding: 1px;
}

.zoom-btn {
  background: transparent;
  border: none;
  padding: 4px 8px;
  font-size: 12px;
  cursor: pointer;
  color: var(--text-primary, #0f172a);
  border-radius: 4px;
}

.zoom-btn:hover {
  background: var(--bg-hover, #e2e8f0);
}

.zoom-label {
  font-size: 11px;
  font-weight: 700;
  padding: 0 6px;
  color: var(--text-secondary, #475569);
  cursor: pointer;
}

/* Explain Button */
.explain-btn {
  background: linear-gradient(135deg, #8b5cf6 0%, #6d28d9 100%);
  color: white;
  border: none;
  border-radius: 8px;
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 6px;
  box-shadow: 0 2px 6px rgba(139, 92, 246, 0.3);
  transition: all 0.15s;
}

.explain-btn:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 10px rgba(139, 92, 246, 0.4);
}

.explain-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* Sentence Ticker Bar */
.sentence-ticker {
  background: var(--ticker-bg, #f8fafc);
  border-top: 1px solid var(--border-color, #e2e8f0);
  padding: 5px 16px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  overflow: hidden;
}

.ticker-badge {
  font-weight: 700;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
  padding: 2px 6px;
  border-radius: 4px;
  white-space: nowrap;
}

.ticker-text {
  flex: 1;
  color: var(--text-primary, #1e293b);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-style: italic;
}

.ticker-progress {
  font-size: 11px;
  color: var(--text-muted, #94a3b8);
  white-space: nowrap;
  font-weight: 600;
}
</style>
