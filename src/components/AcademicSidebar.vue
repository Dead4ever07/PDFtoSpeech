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
          :class="{ active: activeSidebarTab === 'pages' }"
          @click="activeSidebarTab = 'pages'"
        >
          <span>📑 Pages</span>
        </button>
      </div>

      <button class="close-sidebar-btn" @click="isSidebarOpen = false" title="Close Sidebar">
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

      <!-- Tab 2: Fast Page Grid Navigation -->
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
  jumpToSentence,
  jumpToPage
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
