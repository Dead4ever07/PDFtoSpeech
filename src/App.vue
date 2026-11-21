<template>
  <main class="app-layout" :data-theme="theme">
    <CustomTitlebar />
    <ReaderToolbar />
    <div class="workspace-area">
      <PdfPageViewer />
      <AcademicSidebar v-if="isSidebarOpen" />
    </div>
  </main>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';
import CustomTitlebar from './components/CustomTitlebar.vue';
import ReaderToolbar from './components/ReaderToolbar.vue';
import PdfPageViewer from './components/PdfPageViewer.vue';
import AcademicSidebar from './components/AcademicSidebar.vue';
import { usePdfReader } from './composables/usePdfReader';

const {
  theme,
  isSidebarOpen,
  prevPage,
  nextPage,
  prevSentence,
  nextSentence,
  toggleReading,
  stopReading,
  zoomIn,
  zoomOut,
  resetZoom,
  openFile
} = usePdfReader();

// Academic Keyboard Shortcuts
const handleKeydown = (event: KeyboardEvent) => {
  // Ignore key presses if typing in an input field
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
    return;
  }

  // Ctrl+O or Cmd+O for open file
  if ((event.ctrlKey || event.metaKey) && (event.key === 'o' || event.key === 'O')) {
    event.preventDefault();
    openFile();
    return;
  }

  switch (event.key) {
    case ' ':
      event.preventDefault();
      toggleReading();
      break;
    case 'ArrowLeft':
    case 'PageUp':
      event.preventDefault();
      prevPage();
      break;
    case 'ArrowRight':
    case 'PageDown':
      event.preventDefault();
      nextPage();
      break;
    case 'ArrowUp':
      event.preventDefault();
      prevSentence();
      break;
    case 'ArrowDown':
      event.preventDefault();
      nextSentence();
      break;
    case 'Escape':
      event.preventDefault();
      stopReading();
      break;
    case '+':
    case '=':
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        zoomIn();
      }
      break;
    case '-':
    case '_':
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        zoomOut();
      }
      break;
    case '0':
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        resetZoom();
      }
      break;
  }
};

onMounted(() => {
  window.addEventListener('keydown', handleKeydown);
  document.documentElement.setAttribute('data-theme', theme.value);
});

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown);
});
</script>

<style>
/* Global CSS Reset & Academic Typography */
*, *::before, *::after {
  box-sizing: border-box;
}

html, body {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

#app {
  width: 100%;
  height: 100%;
}

/* ============================================================================
   Theme Definitions: Light, Sepia (Warm Paper), Dark
   ============================================================================ */

:root, [data-theme="light"] {
  color-scheme: light;
  --bg-primary: #f8fafc;
  --viewer-bg: #f1f5f9;
  --titlebar-bg: #ffffff;
  --toolbar-bg: #ffffff;
  --sidebar-bg: #ffffff;
  --card-bg: #ffffff;
  --badge-bg: #f1f5f9;
  --ticker-bg: #f8fafc;
  --dropdown-bg: #ffffff;
  --bg-hover: #f1f5f9;
  --overlay-bg: rgba(255, 255, 255, 0.85);
  --spinner-track: #e2e8f0;
  --pdf-page-bg: #ffffff;
  --pdf-filter: none;

  --border-color: #e2e8f0;
  --text-primary: #0f172a;
  --text-secondary: #475569;
  --text-muted: #94a3b8;

  --accent-color: #2563eb;
  --accent-light: #eff6ff;
  --accent-text: #2563eb;

  --active-sentence-bg: #fffbeb;
  --active-pill-bg: #fef3c7;
  --active-pill-text: #92400e;

  --highlight-bg: rgba(245, 158, 11, 0.38);
  --highlight-glow: rgba(245, 158, 11, 0.65);
}

[data-theme="sepia"] {
  color-scheme: light;
  --bg-primary: #fbf0d9;
  --viewer-bg: #f4e8cf;
  --titlebar-bg: #f7eed8;
  --toolbar-bg: #faf3e3;
  --sidebar-bg: #fbf3e4;
  --card-bg: #fefcf7;
  --badge-bg: #eee0c5;
  --ticker-bg: #f5e9d3;
  --dropdown-bg: #fbf3e4;
  --bg-hover: #eedec0;
  --overlay-bg: rgba(251, 240, 217, 0.88);
  --spinner-track: #e2d1b3;
  --pdf-page-bg: #fefcf7;
  --pdf-filter: sepia(0.18) brightness(0.97);

  --border-color: #e2d1b3;
  --text-primary: #3b2c1a;
  --text-secondary: #5c4731;
  --text-muted: #8c7358;

  --accent-color: #b45309;
  --accent-light: #fef3c7;
  --accent-text: #b45309;

  --active-sentence-bg: #fef3c7;
  --active-pill-bg: #fef3c7;
  --active-pill-text: #92400e;

  --highlight-bg: rgba(217, 119, 6, 0.35);
  --highlight-glow: rgba(217, 119, 6, 0.6);
}

[data-theme="dark"] {
  color-scheme: dark;
  --bg-primary: #0f172a;
  --viewer-bg: #0b1120;
  --titlebar-bg: #1e293b;
  --toolbar-bg: #1e293b;
  --sidebar-bg: #1e293b;
  --card-bg: #1e293b;
  --badge-bg: #334155;
  --ticker-bg: #172133;
  --dropdown-bg: #1e293b;
  --bg-hover: #334155;
  --overlay-bg: rgba(15, 23, 42, 0.88);
  --spinner-track: #334155;
  --pdf-page-bg: #1e1e1e;
  --pdf-filter: invert(0.88) hue-rotate(180deg);

  --border-color: #334155;
  --text-primary: #f8fafc;
  --text-secondary: #cbd5e1;
  --text-muted: #94a3b8;

  --accent-color: #3b82f6;
  --accent-light: #1e3a8a;
  --accent-text: #93c5fd;

  --active-sentence-bg: rgba(245, 158, 11, 0.16);
  --active-pill-bg: rgba(245, 158, 11, 0.22);
  --active-pill-text: #fcd34d;

  --highlight-bg: rgba(251, 191, 36, 0.42);
  --highlight-glow: rgba(251, 191, 36, 0.75);
}

/* App Main Layout */
.app-layout {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background-color: var(--bg-primary);
  color: var(--text-primary);
}

.workspace-area {
  flex: 1;
  display: flex;
  overflow: hidden;
  position: relative;
}
</style>
