// Velox Build Pipeline Optimization & Installer Technology Audit — report generator
// Skill chain applied: docx SKILL.md -> routes/create.md -> scenes/report.md (analysis, Template A)
// -> references/design-system.md (R1 cover recipe, DS-1 palette) -> references/common-rules.md (Profile A)
// -> references/docx-js-core.md -> references/toc.md
const {
  Document, Packer, Paragraph, TextRun, Table, TableRow, TableCell,
  PageBreak, Header, Footer, PageNumber, NumberFormat, SectionType,
  AlignmentType, HeadingLevel, WidthType, BorderStyle, ShadingType,
  TableOfContents, TableLayoutType, LevelFormat,
} = require("docx");
const fs = require("fs");

// ─── palette (DS-1 Deep Sea — general business/engineering report) ───
const P = {
  bg: "0B1C2C", primary: "FFFFFF", accent: "529286",
  cover: { titleColor: "FFFFFF", subtitleColor: "B0B8C0", metaColor: "90989F", footerColor: "687078" },
  table: { headerBg: "529286", headerText: "FFFFFF", accentLine: "529286", innerLine: "BECFCC", surface: "E8ECEB" },
  body: "1C2A3D", secondary: "5B6B7D",
};

const NB = { style: BorderStyle.NONE, size: 0, color: "FFFFFF" };
const noBorders = { top: NB, bottom: NB, left: NB, right: NB };
const allNoBorders = { top: NB, bottom: NB, left: NB, right: NB, insideHorizontal: NB, insideVertical: NB };

// ─── design-system helpers (copied verbatim per skill) ───
function splitTitleLines(title, charsPerLine) {
  if (title.length <= charsPerLine) return [title];
  const breakAfter = new Set([..."，。、；：！？", ..."的与和及之在于为", ...",;:!? ", "_-—/"]);
  const lines = [];
  let rest = title;
  while (rest.length > charsPerLine) {
    let cut = -1;
    for (let i = Math.min(charsPerLine, rest.length - 1); i >= Math.floor(charsPerLine * 0.6); i--) {
      if (breakAfter.has(rest[i - 1])) { cut = i; break; }
    }
    if (cut === -1) cut = charsPerLine;
    lines.push(rest.slice(0, cut));
    rest = rest.slice(cut);
  }
  if (rest.length <= 2 && lines.length) lines[lines.length - 1] += rest;
  else if (rest.length) lines.push(rest);
  return lines;
}
function calcTitleLayout(title, maxWidthTwips, preferredPt = 40, minPt = 24) {
  const charsPerLine = (pt) => Math.floor(maxWidthTwips / (pt * 20));
  let titlePt = preferredPt, lines;
  while (titlePt >= minPt) {
    const cpl = charsPerLine(titlePt);
    if (cpl < 2) { titlePt -= 2; continue; }
    lines = splitTitleLines(title, cpl);
    if (lines.length <= 3) break;
    titlePt -= 2;
  }
  if (!lines || lines.length > 3) { lines = splitTitleLines(title, charsPerLine(minPt)); titlePt = minPt; }
  return { titlePt, titleLines: lines };
}
function calcCoverSpacing(params) {
  const { titleLineCount = 1, titlePt = 36, hasSubtitle = false, hasEnglishLabel = false,
    metaLineCount = 0, fixedHeight = 800, pageHeight = 16838, marginTop = 0, marginBottom = 0 } = params;
  const SAFETY = 1200;
  const usableHeight = pageHeight - marginTop - marginBottom - SAFETY;
  const titleHeight = titleLineCount * (titlePt * 23 + 200);
  const subtitleHeight = hasSubtitle ? (12 * 23 + 600) : 0;
  const englishLabelHeight = hasEnglishLabel ? (9 * 23 + 600) : 0;
  const metaHeight = metaLineCount * (10 * 23 + 100);
  const implicitParaHeight = 3 * 300;
  const contentHeight = titleHeight + subtitleHeight + englishLabelHeight + metaHeight + fixedHeight + implicitParaHeight;
  const safeRemaining = Math.max(usableHeight - contentHeight, 400);
  const FOOTER_MIN = 800;
  const rawTop = Math.floor(safeRemaining * 0.45);
  const rawBottom = Math.floor(safeRemaining * 0.45);
  const bottomSpacing = Math.max(rawBottom, FOOTER_MIN);
  const topSpacing = Math.max(rawTop - Math.max(0, FOOTER_MIN - rawBottom), 400);
  return { topSpacing, bottomSpacing };
}

// ─── R1 cover recipe (design-system.md, verbatim structure) ───
function buildCoverR1(config) {
  const pal = config.palette;
  const padL = 1200, padR = 800;
  const availableWidth = 11906 - padL - padR - 300;
  const { titlePt, titleLines } = calcTitleLayout(config.title, availableWidth, 40, 24);
  const titleSize = titlePt * 2;
  const spacing = calcCoverSpacing({
    titleLineCount: titleLines.length, titlePt,
    hasSubtitle: !!config.subtitle, hasEnglishLabel: !!config.englishLabel,
    metaLineCount: (config.metaLines || []).length, fixedHeight: 400,
  });
  const accentLeft = { style: BorderStyle.SINGLE, size: 8, color: pal.accent, space: 12 };
  const children = [];
  children.push(new Paragraph({ spacing: { before: spacing.topSpacing } }));
  if (config.englishLabel) {
    children.push(new Paragraph({
      indent: { left: padL, right: padR }, spacing: { after: 500 },
      border: { bottom: { style: BorderStyle.SINGLE, size: 6, color: pal.accent, space: 8 } },
      children: [new TextRun({ text: config.englishLabel.split("").join("  "), size: 18, color: pal.accent,
        font: { ascii: "Calibri", eastAsia: "SimHei" }, characterSpacing: 40 })],
    }));
  }
  for (let i = 0; i < titleLines.length; i++) {
    children.push(new Paragraph({
      indent: { left: padL },
      spacing: { after: i < titleLines.length - 1 ? 100 : 300, line: Math.ceil(titlePt * 23), lineRule: "atLeast" },
      children: [new TextRun({ text: titleLines[i], size: titleSize, bold: true,
        color: pal.cover.titleColor, font: { eastAsia: "SimHei", ascii: "Arial" } })],
    }));
  }
  if (config.subtitle) {
    children.push(new Paragraph({
      indent: { left: padL }, spacing: { after: 800 },
      children: [new TextRun({ text: config.subtitle, size: 24, color: pal.cover.subtitleColor,
        font: { eastAsia: "Microsoft YaHei", ascii: "Arial" } })],
    }));
  }
  for (const line of (config.metaLines || [])) {
    children.push(new Paragraph({
      indent: { left: padL + 200 }, spacing: { after: 80 },
      border: { left: accentLeft },
      children: [new TextRun({ text: line, size: 24, color: pal.cover.metaColor,
        font: { eastAsia: "Microsoft YaHei", ascii: "Arial" } })],
    }));
  }
  children.push(new Paragraph({ spacing: { before: spacing.bottomSpacing } }));
  children.push(new Paragraph({
    indent: { left: padL, right: padR },
    border: { top: { style: BorderStyle.SINGLE, size: 2, color: pal.accent, space: 8 } },
    spacing: { before: 200 },
    children: [
      new TextRun({ text: config.footerLeft || "", size: 16, color: pal.cover.footerColor, font: { ascii: "Arial" } }),
      new TextRun({ text: "                                        " }),
      new TextRun({ text: config.footerRight || "", size: 16, color: pal.cover.footerColor, font: { ascii: "Arial" } }),
    ],
  }));
  return [new Table({
    width: { size: 100, type: WidthType.PERCENTAGE },
    layout: TableLayoutType.FIXED,
    borders: allNoBorders,
    rows: [new TableRow({
      height: { value: 16838, rule: "exact" },
      children: [new TableCell({
        shading: { type: ShadingType.CLEAR, fill: pal.bg }, borders: noBorders,
        verticalAlign: "top",
        children,
      })],
    })],
  })];
}

// ─── body building blocks (Profile A formal) ───
function h1(text) {
  return new Paragraph({ heading: HeadingLevel.HEADING_1, spacing: { before: 360, after: 160, line: 312 },
    children: [new TextRun({ text, bold: true, size: 32, color: "0B1220", font: { ascii: "Times New Roman", eastAsia: "SimHei" } })] });
}
function h2(text) {
  return new Paragraph({ heading: HeadingLevel.HEADING_2, spacing: { before: 240, after: 120, line: 312 },
    children: [new TextRun({ text, bold: true, size: 28, color: "0B1220", font: { ascii: "Times New Roman", eastAsia: "SimHei" } })] });
}
function body(runs, opts = {}) {
  const children = (typeof runs === "string")
    ? [new TextRun({ text: runs, size: 24, color: "000000", font: { ascii: "Times New Roman", eastAsia: "SimSun" } })]
    : runs;
  return new Paragraph({ alignment: AlignmentType.JUSTIFIED, spacing: { after: 120, line: 312 },
    indent: opts.noIndent ? undefined : { firstLine: 480 }, ...opts.para, children });
}
function b(text) { return new TextRun({ text, bold: true, size: 24, color: "000000", font: { ascii: "Times New Roman", eastAsia: "SimSun" } }); }
function t(text) { return new TextRun({ text, size: 24, color: "000000", font: { ascii: "Times New Roman", eastAsia: "SimSun" } }); }
function code(text) { return new TextRun({ text, size: 21, color: "1C2A3D", font: { ascii: "Courier New", eastAsia: "SimSun" } }); }

function dataTable(caption, header, rows, widths) {
  const captionPara = new Paragraph({ keepNext: true, spacing: { before: 200, after: 100 },
    children: [new TextRun({ text: caption, bold: true, size: 21, color: P.body, font: { ascii: "Times New Roman", eastAsia: "SimHei" } })] });
  const mk = (text, isHeader, alignRight) => new TableCell({
    children: [new Paragraph({ alignment: alignRight ? AlignmentType.RIGHT : AlignmentType.LEFT,
      spacing: { line: 276 },
      children: [new TextRun({ text: String(text), bold: isHeader, size: 20,
        color: isHeader ? P.table.headerText : "000000",
        font: { ascii: "Times New Roman", eastAsia: isHeader ? "SimHei" : "SimSun" } })] })],
    shading: isHeader ? { type: ShadingType.CLEAR, fill: P.table.headerBg } : undefined,
    margins: { top: 60, bottom: 60, left: 120, right: 120 },
    width: undefined,
  });
  const table = new Table({
    width: { size: 100, type: WidthType.PERCENTAGE },
    borders: {
      top: { style: BorderStyle.SINGLE, size: 2, color: P.table.accentLine },
      bottom: { style: BorderStyle.SINGLE, size: 2, color: P.table.accentLine },
      left: NB, right: NB,
      insideHorizontal: { style: BorderStyle.SINGLE, size: 1, color: P.table.innerLine },
      insideVertical: NB,
    },
    rows: [
      new TableRow({ tableHeader: true, cantSplit: true,
        children: header.map((h, i) => { const c = mk(h, true, false); return c; }) }),
      ...rows.map((r, ri) => new TableRow({ cantSplit: true,
        children: r.map((cell, i) => {
          const c = mk(cell, false, typeof cell === "string" && /^[0-9][0-9.,]*\s*(s|MB|bytes|%|x| crates)?$/.test(cell.trim()));
          if (ri % 2 === 1) c.root.push; return c;
        }) })),
    ],
  });
  return [captionPara, table];
}

// zebra striping via a second pass is complex in docx-js; instead use explicit shading on odd rows:
function dataTable2(caption, header, rows, rightAlignCols = []) {
  const captionPara = new Paragraph({ keepNext: true, spacing: { before: 200, after: 100 },
    children: [new TextRun({ text: caption, bold: true, size: 21, color: P.body, font: { ascii: "Times New Roman", eastAsia: "SimHei" } })] });
  const mkCell = (text, isHeader, ri, ci) => new TableCell({
    children: [new Paragraph({
      alignment: rightAlignCols.includes(ci) && !isHeader ? AlignmentType.RIGHT : AlignmentType.LEFT,
      spacing: { line: 276 },
      children: [new TextRun({ text: String(text), bold: isHeader, size: 20,
        color: isHeader ? P.table.headerText : "000000",
        font: { ascii: "Times New Roman", eastAsia: isHeader ? "SimHei" : "SimSun" } })] })],
    shading: isHeader ? { type: ShadingType.CLEAR, fill: P.table.headerBg }
      : (ri % 2 === 1 ? { type: ShadingType.CLEAR, fill: P.table.surface } : undefined),
    margins: { top: 60, bottom: 60, left: 120, right: 120 },
  });
  const table = new Table({
    width: { size: 100, type: WidthType.PERCENTAGE },
    borders: {
      top: { style: BorderStyle.SINGLE, size: 2, color: P.table.accentLine },
      bottom: { style: BorderStyle.SINGLE, size: 2, color: P.table.accentLine },
      left: NB, right: NB,
      insideHorizontal: { style: BorderStyle.SINGLE, size: 1, color: P.table.innerLine },
      insideVertical: NB,
    },
    rows: [
      new TableRow({ tableHeader: true, cantSplit: true, children: header.map((h, ci) => mkCell(h, true, 0, ci)) }),
      ...rows.map((r, ri) => new TableRow({ cantSplit: true, children: r.map((c, ci) => mkCell(c, false, ri, ci)) })),
    ],
  });
  return [captionPara, table];
}

module.exports = { Document, Packer, Paragraph, TextRun, Table, TableRow, TableCell, PageBreak, Header, Footer,
  PageNumber, NumberFormat, SectionType, AlignmentType, HeadingLevel, WidthType, BorderStyle, ShadingType,
  TableOfContents, TableLayoutType, LevelFormat, fs, P, buildCoverR1, h1, h2, body, b, t, code, dataTable2 };
