<template>
  <div data-tauri-drag-region class="titlebar">
    <div class="left-section">
      <div class="app-brand" @click="handleOpenFile">
        <span class="brand-icon">📚</span>
        <span class="brand-title">PDFtoSpeech</span>
      </div>

      <div class="menu-item" @click="toggleFileMenu">
        <span>File ▾</span>
        <div v-if="isFileMenuOpen" class="dropdown-menu">
          <div class="dropdown-item" @click.stop="handleOpenFile">
            <span>📄</span> Open PDF Document...
          </div>
          <div class="dropdown-separator"></div>
          <div class="dropdown-item" @click.stop="toggleSidebar('transcript')">
            <span>📝</span> Toggle Transcript
          </div>
          <div class="dropdown-item" @click.stop="toggleSidebar('explain')">
            <span>💡</span> Explanatory Insights
          </div>
        </div>
      </div>

      <button class="quick-open-btn" @click="handleOpenFile" title="Open PDF (Ctrl+O)">
        <span class="btn-icon">📁</span> Open PDF
      </button>
    </div>

    <!-- Center: Document Title & Current Page -->
    <div data-tauri-drag-region class="center-section">
      <div v-if="documentName" class="doc-badge">
        <span class="doc-icon">📑</span>
        <span class="doc-title" :title="documentName">{{ documentName }}</span>
        <span class="doc-page-badge" v-if="totalPages > 0">p. {{ currentPageIndex + 1 }} / {{ totalPages }}</span>
      </div>
      <div v-else class="app-tagline">
        Academic & Doctoral Thesis Reader
      </div>
    </div>

    <!-- Right: Theme, Sidebar Toggle, and Window Controls -->
    <div class="right-section">
      <!-- Theme Switcher -->
      <div class="theme-switcher" title="Reading Theme">
        <button
          class="theme-btn"
          :class="{ active: theme === 'light' }"
          @click="setTheme('light')"
          title="Light Paper Mode"
        >
          ☀️
        </button>
        <button
          class="theme-btn"
          :class="{ active: theme === 'sepia' }"
          @click="setTheme('sepia')"
          title="Sepia Reading Mode (Warm)"
        >
          📜
        </button>
        <button
          class="theme-btn"
          :class="{ active: theme === 'dark' }"
          @click="setTheme('dark')"
          title="Dark Mode"
        >
          🌙
        </button>
      </div>

      <!-- Sidebar Toggle -->
      <button
        v-if="totalPages > 0"
        class="sidebar-toggle-btn"
        :class="{ active: isSidebarOpen }"
        @click="isSidebarOpen = !isSidebarOpen"
        title="Toggle Academic Assistant Panel"
      >
        <span>💡 Assistant</span>
      </button>

      <!-- Standard Window Controls -->
      <div class="window-controls">
        <button class="control-btn minimize" @click="minimize" title="Minimize">─</button>
        <button class="control-btn maximize" @click="toggleMaximize" title="Maximize">□</button>
        <button class="control-btn close" @click="close" title="Close">✕</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { usePdfReader } from '../composables/usePdfReader';

const appWindow = getCurrentWindow();
const {
  openFile,
  documentName,
  currentPageIndex,
  totalPages,
  theme,
  setTheme,
  isSidebarOpen,
  activeSidebarTab
} = usePdfReader();

const minimize = () => appWindow.minimize();
const toggleMaximize = () => appWindow.toggleMaximize();
const close = () => appWindow.close();

const isFileMenuOpen = ref(false);

const toggleFileMenu = (event: Event) => {
  event.stopPropagation();
  isFileMenuOpen.value = !isFileMenuOpen.value;
};

const handleOpenFile = async () => {
  isFileMenuOpen.value = false;
  await openFile();
};

const toggleSidebar = (tab: 'transcript' | 'explain') => {
  isFileMenuOpen.value = false;
  activeSidebarTab.value = tab;
  isSidebarOpen.value = true;
};

const closeMenu = () => {
  isFileMenuOpen.value = false;
};

onMounted(() => window.addEventListener('click', closeMenu));
onUnmounted(() => window.removeEventListener('click', closeMenu));
</script>

<style scoped>
.titlebar {
  height: 42px;
  background-color: var(--titlebar-bg, #ffffff);
  border-bottom: 1px solid var(--border-color, #e2e8f0);
  display: flex;
  justify-content: space-between;
  align-items: center;
  user-select: none;
  font-family: inherit;
  font-size: 13px;
  z-index: 100;
  transition: background-color 0.2s, border-color 0.2s;
}

.titlebar[data-tauri-drag-region] {
  cursor: default;
}

.left-section,
.right-section {
  display: flex;
  align-items: center;
  height: 100%;
}

.left-section {
  padding-left: 10px;
  gap: 8px;
}

.app-brand {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-weight: 700;
  color: var(--text-primary, #0f172a);
}

.app-brand:hover {
  background-color: var(--bg-hover, #f1f5f9);
}

.brand-icon {
  font-size: 16px;
}

.brand-title {
  font-size: 14px;
  letter-spacing: -0.3px;
}

.menu-item {
  position: relative;
  display: flex;
  align-items: center;
  padding: 4px 10px;
  border-radius: 6px;
  cursor: pointer;
  color: var(--text-secondary, #475569);
  font-weight: 500;
}

.menu-item:hover {
  background-color: var(--bg-hover, #f1f5f9);
  color: var(--text-primary, #0f172a);
}

.dropdown-menu {
  position: absolute;
  top: 100%;
  left: 0;
  margin-top: 4px;
  background: var(--dropdown-bg, #ffffff);
  border: 1px solid var(--border-color, #e2e8f0);
  box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.12), 0 8px 10px -6px rgba(0, 0, 0, 0.08);
  border-radius: 8px;
  padding: 6px;
  min-width: 200px;
  z-index: 200;
}

.dropdown-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  font-size: 13px;
  border-radius: 6px;
  cursor: pointer;
  color: var(--text-primary, #0f172a);
  transition: background 0.12s;
}

.dropdown-item:hover {
  background-color: var(--accent-light, #eff6ff);
  color: var(--accent-color, #2563eb);
}

.dropdown-separator {
  height: 1px;
  background: var(--border-color, #e2e8f0);
  margin: 4px 0;
}

.quick-open-btn {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px;
  border: 1px solid var(--border-color, #cbd5e1);
  background: var(--card-bg, #ffffff);
  color: var(--text-primary, #0f172a);
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}

.quick-open-btn:hover {
  border-color: var(--accent-color, #2563eb);
  color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
}

.center-section {
  flex: 1;
  display: flex;
  justify-content: center;
  align-items: center;
  pointer-events: none;
  overflow: hidden;
  padding: 0 15px;
}

.doc-badge {
  display: flex;
  align-items: center;
  gap: 8px;
  max-width: 480px;
  background: var(--badge-bg, #f1f5f9);
  padding: 3px 12px;
  border-radius: 12px;
  border: 1px solid var(--border-color, #e2e8f0);
}

.doc-icon {
  font-size: 14px;
}

.doc-title {
  font-weight: 600;
  font-size: 12px;
  color: var(--text-primary, #0f172a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.doc-page-badge {
  font-size: 11px;
  font-weight: 700;
  color: var(--accent-color, #2563eb);
  background: var(--accent-light, #eff6ff);
  padding: 1px 6px;
  border-radius: 6px;
}

.app-tagline {
  font-size: 12px;
  color: var(--text-muted, #94a3b8);
  font-style: italic;
}

.right-section {
  gap: 8px;
}

.theme-switcher {
  display: flex;
  background: var(--badge-bg, #f1f5f9);
  padding: 2px;
  border-radius: 8px;
  border: 1px solid var(--border-color, #e2e8f0);
}

.theme-btn {
  background: transparent;
  border: none;
  padding: 4px 6px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
  line-height: 1;
  transition: all 0.15s;
}

.theme-btn:hover {
  background: var(--bg-hover, #e2e8f0);
}

.theme-btn.active {
  background: var(--card-bg, #ffffff);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
}

.sidebar-toggle-btn {
  background: var(--card-bg, #ffffff);
  border: 1px solid var(--border-color, #cbd5e1);
  padding: 5px 10px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-primary, #0f172a);
  cursor: pointer;
  transition: all 0.15s;
}

.sidebar-toggle-btn:hover {
  border-color: #f59e0b;
  color: #d97706;
}

.sidebar-toggle-btn.active {
  background: #fef3c7;
  border-color: #f59e0b;
  color: #92400e;
}

.window-controls {
  display: flex;
  height: 100%;
}

.control-btn {
  display: flex;
  justify-content: center;
  align-items: center;
  width: 44px;
  height: 100%;
  border: none;
  background: transparent;
  cursor: pointer;
  color: var(--text-secondary, #64748b);
  font-size: 13px;
  transition: background 0.12s, color 0.12s;
}

.control-btn:hover {
  background-color: var(--bg-hover, #e2e8f0);
  color: var(--text-primary, #0f172a);
}

.control-btn.close:hover {
  background-color: #ef4444;
  color: #ffffff;
}
</style>
