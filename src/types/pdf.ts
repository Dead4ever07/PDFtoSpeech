export interface PdfTextElement {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  layout_label?: string | null;
  is_filtered?: boolean;
}

export interface PdfPageData {
  page_number: number;
  width: number;
  height: number;
  elements: PdfTextElement[];
}

export interface SentenceBlock {
  text: string;
  cleanedText?: string;
  elementIndices: number[]; // Keeps track of which visual lines belong to this sentence
  startsParagraph?: boolean;
}

export type ReadingMode = 'verbatim' | 'clean';
export type AppTheme = 'light' | 'sepia' | 'dark';
