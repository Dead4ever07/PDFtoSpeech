<template>
  <aside class="sidebar-container" :class="{ 'is-open': isSidebarOpen }">
    <div class="sidebar-header">
      <div class="tabs">
        <button
          class="tab-btn"
          :class="{ active: activeSidebarTab === 'transcript' }"
          @click="activeSidebarTab = 'transcript'"
        >
          <span>📝 Transcript</span>
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeSidebarTab === 'explain' }"
          @click="activeSidebarTab = 'explain'"
        >
          <span>💡 Insights</span>
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeSidebarTab === 'pages' }"
          @click="activeSidebarTab = 'pages'"
        >
          <span>📑 Pages</span>
        </button>
      </div>

      <button class="close-sidebar-btn" @click="isSidebarOpen = false" title="Close Assistant">
        ✕
      </button>
    </div>

    <div class="sidebar-body">

      <!-- Tab 1: Live Synchronized Transcript -->
      <div v-if="activeSidebarTab === 'transcript'" class="transcript-view">
        <div class="section-title">
          <span>Page {{ currentPageIndex + 1 }} Reading Stream</span>
          <span class="badge">{{ pageSentences.length }} Sentences</span>
        </div>

        <div v-if="pageSentences.length === 0" class="empty-tab">
          <p>No readable text elements found on this page.</p>
        </div>

        <div v-else class="sentences-list">
          <div
            v-for="(sentence, idx) in pageSentences"
            :key="idx"
            class="sentence-item"
            :class="{ active: currentSentenceIndex === idx }"
            @click="jumpToSentence(idx)"
          >
            <span class="sentence-num">{{ idx + 1 }}</span>
            <div class="sentence-content">
              <p class="sentence-text">{{ sentence.text }}</p>
              <p v-if="readingMode === 'clean' && sentence.cleanedText !== sentence.text" class="cleaned-preview">
                <span class="preview-tag">TTS Readout:</span> {{ sentence.cleanedText }}
              </p>
            </div>
          </div>
        </div>
      </div>

      <!-- Tab 2: Explanatory Assistant / Dissertation Demystifier -->
      <div v-else-if="activeSidebarTab === 'explain'" class="explain-view">
        <div class="section-title">
          <span>Doctoral Thesis Explainer</span>
        </div>

        <div class="explain-card hero-card">
          <h4>💡 Research Translation</h4>
          <p class="card-desc">
            Translates dense academic prose, complex formulas, and methodological jargon into intuitive, plain concepts.
          </p>

          <button
            class="action-explain-btn"
            :class="{ loading: isExplaining }"
            @click="requestExplanation"
            :disabled="currentSentenceIndex === -1"
          >
            <span v-if="isExplaining">Synthesizing Explanation...</span>
            <span v-else>🔍 Unpack Active Sentence</span>
          </button>
        </div>

        <div v-if="currentExplanation" class="explanation-results">
          <!-- Core Takeaway -->
          <div class="insight-block">
            <h5>🎯 Core Claim / Takeaway</h5>
            <p>{{ currentExplanation.summary }}</p>
          </div>

          <!-- Intuition / Analogy -->
          <div class="insight-block intuition-block">
            <h5>🧩 Plain-English Intuition</h5>
            <p>{{ currentExplanation.intuition }}</p>
          </div>

          <!-- Demystified Jargon -->
          <div v-if="currentExplanation.terms.length > 0" class="insight-block">
            <h5>📖 Academic Terminology</h5>
            <div class="terms-list">
              <div v-for="(t, idx) in currentExplanation.terms" :key="idx" class="term-item">
                <span class="term-badge">{{ t.term }}</span>
                <span class="term-def">{{ t.definition }}</span>
              </div>
            </div>
          </div>
        </div>

        <div v-else class="explain-empty">
          <span class="empty-icon">🎓</span>
          <p>Select any sentence or click <strong>Unpack Active Sentence</strong> to generate an explanatory breakdown of the dissertation argument.</p>
        </div>

        <!-- Academic Reader Tips -->
        <div class="academic-tips">
          <h6>Academic Reading Tips:</h6>
          <ul>
            <li><strong>Clean Mode:</strong> Automatically strips citation noise like <code>[1-3]</code> and <code>(Author, 2022)</code>.</li>
            <li><strong>Click to Listen:</strong> Click any sentence on the PDF page to immediately play audio.</li>
            <li><strong>Speed Adjustment:</strong> Speed up literature reviews to 1.35x–1.5x to absorb content faster.</li>
          </ul>
        </div>
      </div>

      <!-- Tab 3: Fast Page Grid Navigation -->
      <div v-else-if="activeSidebarTab === 'pages'" class="pages-view">
        <div class="section-title">
          <span>Document Navigation</span>
          <span class="badge">{{ totalPages }} Pages</span>
        </div>

        <div class="page-grid">
          <button
            v-for="p in totalPages"
            :key="p"
            class="page-tile"
            :class="{ active: currentPageIndex === p - 1 }"
            @click="jumpToPage(p)"
          >
            <span class="tile-number">p. {{ p }}</span>
          </button>
        </div>
      </div>

    </div>
  </aside>
</template>

<script setup lang="ts">
import { usePdfReader } from '../composables/usePdfReader';

const {
  currentPageIndex,
  totalPages,
  currentSentenceIndex,
  pageSentences,
  readingMode,
  isSidebarOpen,
  activeSidebarTab,
  isExplaining,
  currentExplanation,
  jumpToSentence,
  jumpToPage,
  requestExplanation
} = usePdfReader();
</script>

<style scoped>
.sidebar-container {
  width: 360px;
  max-width: 90vw;
  background: var(--sidebar-bg, #ffffff);
  border-left: 1px solid var(--border-color, #e2e8f0);
  display: flex;
  flex-direction: column;
  height: 100%;
  box-shadow: -4px 0 16px rgba(0, 0, 0, 0.05);
  transition: transform 0.25s ease, margin 0.25s ease;
  z-index: 40;
}

.sidebar-header {
  padding: 8px 12px;
  background: var(--titlebar-bg, #f8fafc);
  border-bottom: 1px solid var(--border-color, #e2e8f0);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.tabs {
  display: flex;
  gap: 4px;
}

.tab-btn {
  background: transparent;
  border: 1px solid transparent;
  padding: 5px 10px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary, #64748b);
  cursor: pointer;
  transition: all 0.15s;
}

.tab-btn:hover {
  color: var(--text-primary, #0f172a);
  background: var(--bg-hover, #f1f5f9);
}

.tab-btn.active {
  background: var(--card-bg, #ffffff);
  border-color: var(--border-color, #cbd5e1);
  color: var(--accent-text, #2563eb);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
}

.close-sidebar-btn {
  border: none;
  background: transparent;
  font-size: 14px;
  color: var(--text-secondary, #64748b);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 4px;
}

.close-sidebar-btn:hover {
  background: var(--bg-hover, #e2e8f0);
  color: var(--text-primary, #0f172a);
}

.sidebar-body {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
}

.section-title {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 14px;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-primary, #0f172a);
}

.badge {
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-text, #2563eb);
  background: var(--accent-light, #eff6ff);
  padding: 2px 8px;
  border-radius: 12px;
}

/* Transcript View */
.sentences-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.sentence-item {
  display: flex;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border-color, #e2e8f0);
  background: var(--card-bg, #ffffff);
  cursor: pointer;
  transition: all 0.15s;
}

.sentence-item:hover {
  border-color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
}

.sentence-item.active {
  border-color: #f59e0b;
  background: var(--active-sentence-bg, #fffbeb);
  box-shadow: 0 0 0 1px #f59e0b;
}

.sentence-num {
  font-size: 11px;
  font-weight: 700;
  color: var(--text-muted, #94a3b8);
  min-width: 18px;
}

.sentence-content {
  flex: 1;
}

.sentence-text {
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-primary, #1e293b);
  margin: 0;
}

.cleaned-preview {
  margin: 4px 0 0 0;
  font-size: 11px;
  color: var(--text-secondary, #64748b);
  font-style: italic;
}

.preview-tag {
  font-weight: 600;
  color: #10b981;
}

/* Explain View */
.hero-card {
  background: var(--hero-card-bg, linear-gradient(135deg, #f5f3ff 0%, #ede9fe 100%));
  border: 1px solid var(--hero-card-border, #ddd6fe);
  padding: 14px;
  border-radius: 10px;
  margin-bottom: 16px;
}

.hero-card h4 {
  margin: 0 0 6px 0;
  font-size: 14px;
  color: var(--hero-card-title, #5b21b6);
}

.card-desc {
  font-size: 12px;
  color: var(--hero-card-desc, #6d28d9);
  line-height: 1.4;
  margin: 0 0 12px 0;
}

.action-explain-btn {
  width: 100%;
  background: linear-gradient(135deg, #7c3aed 0%, #6d28d9 100%);
  color: white;
  border: none;
  padding: 8px 14px;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
  transition: all 0.15s;
  box-shadow: 0 2px 4px rgba(124, 58, 237, 0.25);
}

.action-explain-btn:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 8px rgba(124, 58, 237, 0.35);
}

.action-explain-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.action-explain-btn.loading {
  background: #6d28d9;
}

.explanation-results {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: 16px;
}

.insight-block {
  background: var(--card-bg, #ffffff);
  border: 1px solid var(--border-color, #e2e8f0);
  border-radius: 8px;
  padding: 12px;
}

.insight-block h5 {
  margin: 0 0 6px 0;
  font-size: 12px;
  color: var(--text-primary, #0f172a);
}

.insight-block p {
  margin: 0;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-secondary, #334155);
}

.intuition-block {
  background: var(--intuition-bg, #f0fdf4);
  border-color: var(--intuition-border, #bbf7d0);
}

.intuition-block h5 {
  color: var(--intuition-title, #166534);
}

.intuition-block p {
  color: var(--intuition-text, #14532d);
}

.terms-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 6px;
}

.term-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  background: var(--badge-bg, #f8fafc);
  padding: 6px 8px;
  border-radius: 6px;
}

.term-badge {
  font-weight: 700;
  font-size: 11px;
  color: var(--term-badge-color, #7c3aed);
}

.term-def {
  font-size: 11px;
  color: var(--text-secondary, #475569);
  line-height: 1.3;
}

.explain-empty {
  text-align: center;
  padding: 30px 10px;
  color: var(--text-muted, #94a3b8);
}

.empty-icon {
  font-size: 32px;
  display: block;
  margin-bottom: 8px;
}

.explain-empty p {
  font-size: 12px;
  line-height: 1.5;
  margin: 0;
}

.academic-tips {
  background: var(--badge-bg, #f8fafc);
  border: 1px solid var(--border-color, #e2e8f0);
  border-radius: 8px;
  padding: 12px;
  font-size: 11px;
}

.academic-tips h6 {
  margin: 0 0 6px 0;
  font-size: 11px;
  font-weight: 700;
  color: var(--text-primary, #0f172a);
}

.academic-tips ul {
  margin: 0;
  padding-left: 16px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  color: var(--text-secondary, #475569);
}

/* Page Grid View */
.page-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}

.page-tile {
  background: var(--card-bg, #ffffff);
  color: var(--text-primary, #0f172a);
  border: 1px solid var(--border-color, #cbd5e1);
  padding: 10px 4px;
  border-radius: 6px;
  cursor: pointer;
  text-align: center;
  transition: all 0.12s;
}

.page-tile:hover {
  border-color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
}

.page-tile.active {
  background: var(--accent-color, #2563eb);
  border-color: var(--accent-color, #2563eb);
  color: white;
}

.tile-number {
  font-size: 12px;
  font-weight: 700;
}
</style>
