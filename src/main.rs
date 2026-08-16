// LiteNote v3 — multi-format viewer + editor
// TXT (edit), MD (rendered), DOCX (rendered via JSON), SRT (cards)
// Features: drag & drop, copy all, find highlight, format badge

#![windows_subsystem = "windows"]

use eframe::egui;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ─────────────────────────────────────────────────────────────────────────────
// File type detection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum FileKind {
    PlainText,
    Markdown,
    Docx,
    Srt,
    Binary,
}

fn detect_kind(path: &PathBuf) -> FileKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("md" | "markdown") => FileKind::Markdown,
        Some("docx") => FileKind::Docx,
        Some("srt") => FileKind::Srt,
        Some(
            "pdf" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico"
            | "mp4" | "mp3" | "wav" | "ogg" | "avi" | "mkv" | "mov" | "flac"
            | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "odt" | "ods" | "odp"
            | "zip" | "rar" | "7z" | "tar" | "gz" | "bz2"
            | "exe" | "dll" | "msi" | "bin" | "dat" | "iso",
        ) => FileKind::Binary,
        _ => FileKind::PlainText,
    }
}

fn open_with_default_app(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", &path.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Failed to open: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open: {e}"))?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Document blocks (rendered view)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Block {
    H1(String),
    H2(String),
    H3(String),
    CodeBlock(String),
    Quote(String),
    Hr,
    ListItem(String),
    Paragraph(String),
    SrtEntry { index: u32, time: String, text: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Markdown parser
// ─────────────────────────────────────────────────────────────────────────────

fn parse_markdown(src: &str) -> Vec<Block> {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    let mut blocks: Vec<Block> = Vec::new();
    let parser = Parser::new(src);
    let mut buf = String::new();
    let mut in_code_block = false;
    let mut in_heading: Option<HeadingLevel> = None;
    let mut in_quote = false;
    let mut in_list_item = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(Block::CodeBlock(buf.trim().to_string()));
                in_code_block = false;
                buf.clear();
            }
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(level);
                buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let t = buf.trim().to_string();
                match in_heading {
                    Some(HeadingLevel::H1) => blocks.push(Block::H1(t)),
                    Some(HeadingLevel::H2) => blocks.push(Block::H2(t)),
                    _ => blocks.push(Block::H3(t)),
                }
                in_heading = None;
                buf.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                in_quote = true;
                buf.clear();
            }
            Event::End(TagEnd::BlockQuote) => {
                blocks.push(Block::Quote(buf.trim().to_string()));
                in_quote = false;
                buf.clear();
            }
            Event::Start(Tag::Item) => {
                in_list_item = true;
                buf.clear();
            }
            Event::End(TagEnd::Item) => {
                blocks.push(Block::ListItem(buf.trim().to_string()));
                in_list_item = false;
                buf.clear();
            }
            Event::Start(Tag::Paragraph) => {
                if !in_quote && !in_list_item {
                    buf.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if !in_quote && !in_list_item {
                    let t = buf.trim().to_string();
                    if !t.is_empty() {
                        blocks.push(Block::Paragraph(t));
                    }
                    buf.clear();
                }
            }
            Event::Rule => blocks.push(Block::Hr),
            Event::Text(t) => buf.push_str(&t),
            Event::Code(t) => buf.push_str(&t),
            Event::SoftBreak | Event::HardBreak => buf.push(' '),
            _ => {}
        }
    }
    blocks
}

// ─────────────────────────────────────────────────────────────────────────────
// DOCX parser — via docx-rs JSON output (safe, version-stable)
// ─────────────────────────────────────────────────────────────────────────────

fn parse_docx(path: &PathBuf) -> Result<Vec<Block>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let docx = docx_rs::read_docx(&bytes).map_err(|e| format!("{:?}", e))?;
    let json_str = docx.json();
    let json: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))?;

    let mut blocks = Vec::new();

    let children = json
        .pointer("/document/children")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for child in &children {
        // Each child has a "type" field: "paragraph" or "table"
        let child_type = child
            .pointer("/type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if child_type != "paragraph" {
            continue;
        }

        let data = match child.get("data") {
            Some(d) => d,
            None => continue,
        };

        // Collect all run texts
        let mut text = String::new();
        if let Some(runs) = data.pointer("/children").and_then(|v| v.as_array()) {
            for run in runs {
                let run_type = run.pointer("/type").and_then(|v| v.as_str()).unwrap_or("");
                if run_type != "run" {
                    continue;
                }
                if let Some(run_children) =
                    run.pointer("/data/children").and_then(|v| v.as_array())
                {
                    for rc in run_children {
                        let rc_type = rc.pointer("/type").and_then(|v| v.as_str()).unwrap_or("");
                        if rc_type == "text" {
                            if let Some(t) =
                                rc.pointer("/data/text").and_then(|v| v.as_str())
                            {
                                text.push_str(t);
                            }
                        }
                    }
                }
            }
        }

        if text.trim().is_empty() {
            continue;
        }

        // Check paragraph style
        let style = data
            .pointer("/property/style")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        if style.contains("heading1") || style == "title" {
            blocks.push(Block::H1(text));
        } else if style.contains("heading2") {
            blocks.push(Block::H2(text));
        } else if style.contains("heading3") {
            blocks.push(Block::H3(text));
        } else {
            blocks.push(Block::Paragraph(text));
        }
    }

    Ok(blocks)
}

// ─────────────────────────────────────────────────────────────────────────────
// SRT parser
// ─────────────────────────────────────────────────────────────────────────────

fn parse_srt(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for chunk in src.trim().split("\n\n") {
        let lines: Vec<&str> = chunk.lines().collect();
        if lines.len() < 3 {
            continue;
        }
        let index = lines[0].trim().parse::<u32>().unwrap_or(0);
        let time = lines[1].trim().to_string();
        let text = lines[2..].join("\n");
        blocks.push(Block::SrtEntry { index, time, text });
    }
    blocks
}

// ─────────────────────────────────────────────────────────────────────────────
// App state
// ─────────────────────────────────────────────────────────────────────────────

struct LiteNote {
    content: String,
    blocks: Vec<Block>,
    file_path: Option<PathBuf>,
    file_kind: FileKind,
    modified: bool,
    dark_mode: bool,
    font_size: f32,
    word_wrap: bool,
    status: String,
    status_is_error: bool,
    show_find: bool,
    find_query: String,
    copy_buffer: Option<String>,
}

impl Default for LiteNote {
    fn default() -> Self {
        Self {
            content: String::new(),
            blocks: Vec::new(),
            file_path: None,
            file_kind: FileKind::PlainText,
            modified: false,
            dark_mode: true,
            font_size: 16.0,
            word_wrap: true,
            status: "Ready — drag a file here or click Open".to_string(),
            status_is_error: false,
            show_find: false,
            find_query: String::new(),
            copy_buffer: None,
        }
    }
}

impl LiteNote {
    fn window_title(&self) -> String {
        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        format!("LiteNote — {}{}", name, if self.modified { " *" } else { "" })
    }

    fn load_file(&mut self, path: PathBuf) {
        let kind = detect_kind(&path);

        match kind {
            FileKind::Binary => {
                match open_with_default_app(&path) {
                    Ok(_) => {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        self.status = format!("'{}' opened in default app", name);
                        self.status_is_error = false;
                    }
                    Err(e) => {
                        self.status = e;
                        self.status_is_error = true;
                    }
                }
                return;
            }

            FileKind::Docx => {
                match parse_docx(&path) {
                    Ok(blocks) => {
                        self.blocks = blocks;
                        self.content = String::new();
                        self.file_path = Some(path);
                        self.file_kind = FileKind::Docx;
                        self.modified = false;
                        self.status = "Opened DOCX".to_string();
                        self.status_is_error = false;
                    }
                    Err(e) => {
                        self.status = format!("DOCX error: {e}");
                        self.status_is_error = true;
                    }
                }
                return;
            }

            _ => {}
        }

        // Text-based formats
        match fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes.clone())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());

                self.blocks = match kind {
                    FileKind::Markdown => parse_markdown(&text),
                    FileKind::Srt => parse_srt(&text),
                    _ => Vec::new(),
                };

                self.content = text;
                self.file_kind = kind;
                self.file_path = Some(path);
                self.modified = false;
                self.status_is_error = false;
                self.status = format!(
                    "Opened {}",
                    match self.file_kind {
                        FileKind::Markdown => "Markdown",
                        FileKind::Srt => "SRT Subtitle",
                        _ => "Text file",
                    }
                );
            }
            Err(e) => {
                self.status = format!("Error: {e}");
                self.status_is_error = true;
            }
        }
    }

    fn new_file(&mut self) {
        self.content.clear();
        self.blocks.clear();
        self.file_path = None;
        self.file_kind = FileKind::PlainText;
        self.modified = false;
        self.status = "New file".to_string();
        self.status_is_error = false;
    }

    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            self.load_file(path);
        }
    }

    fn save_file(&mut self) {
        if let Some(path) = self.file_path.clone() {
            match fs::write(&path, &self.content) {
                Ok(_) => {
                    self.modified = false;
                    self.status = "Saved".to_string();
                    self.status_is_error = false;
                }
                Err(e) => {
                    self.status = format!("Error saving: {e}");
                    self.status_is_error = true;
                }
            }
        } else {
            self.save_file_as();
        }
    }

    fn save_file_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new().save_file() {
            match fs::write(&path, &self.content) {
                Ok(_) => {
                    self.file_path = Some(path);
                    self.modified = false;
                    self.status = "Saved".to_string();
                    self.status_is_error = false;
                }
                Err(e) => {
                    self.status = format!("Error saving: {e}");
                    self.status_is_error = true;
                }
            }
        }
    }

    fn all_text(&self) -> String {
        if self.blocks.is_empty() {
            return self.content.clone();
        }
        self.blocks
            .iter()
            .map(|b| match b {
                Block::H1(t) | Block::H2(t) | Block::H3(t) | Block::CodeBlock(t)
                | Block::Quote(t) | Block::ListItem(t) | Block::Paragraph(t) => t.clone(),
                Block::Hr => "─────────────────────────────────────".to_string(),
                Block::SrtEntry { index, time, text } => {
                    format!("{}\n{}\n{}", index, time, text)
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Theme
// ─────────────────────────────────────────────────────────────────────────────

struct Theme {
    toolbar_bg: egui::Color32,
    editor_bg: egui::Color32,
    btn_bg: egui::Color32,
    btn_hover: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
    text_dim: egui::Color32,
    h1: egui::Color32,
    h2: egui::Color32,
    h3: egui::Color32,
    code_bg: egui::Color32,
    quote_bg: egui::Color32,
    srt_card: egui::Color32,
    srt_index: egui::Color32,
    srt_time: egui::Color32,
    green: egui::Color32,
    red: egui::Color32,
}

impl Theme {
    fn dark() -> Self {
        Self {
            toolbar_bg: egui::Color32::from_rgb(18, 21, 40),
            editor_bg: egui::Color32::from_rgb(12, 14, 26),
            btn_bg: egui::Color32::from_rgb(35, 40, 70),
            btn_hover: egui::Color32::from_rgb(55, 62, 105),
            accent: egui::Color32::from_rgb(108, 126, 255),
            text: egui::Color32::from_rgb(215, 220, 245),
            text_dim: egui::Color32::from_rgb(95, 105, 155),
            h1: egui::Color32::from_rgb(148, 168, 255),
            h2: egui::Color32::from_rgb(100, 210, 245),
            h3: egui::Color32::from_rgb(140, 230, 165),
            code_bg: egui::Color32::from_rgb(20, 24, 45),
            quote_bg: egui::Color32::from_rgba_unmultiplied(80, 90, 160, 28),
            srt_card: egui::Color32::from_rgba_unmultiplied(38, 44, 85, 140),
            srt_index: egui::Color32::from_rgb(108, 126, 255),
            srt_time: egui::Color32::from_rgb(88, 210, 165),
            green: egui::Color32::from_rgb(88, 215, 130),
            red: egui::Color32::from_rgb(255, 95, 95),
        }
    }

    fn light() -> Self {
        Self {
            toolbar_bg: egui::Color32::from_rgb(232, 235, 255),
            editor_bg: egui::Color32::from_rgb(250, 251, 255),
            btn_bg: egui::Color32::from_rgb(210, 216, 245),
            btn_hover: egui::Color32::from_rgb(188, 196, 238),
            accent: egui::Color32::from_rgb(65, 85, 215),
            text: egui::Color32::from_rgb(28, 30, 58),
            text_dim: egui::Color32::from_rgb(100, 110, 152),
            h1: egui::Color32::from_rgb(38, 58, 200),
            h2: egui::Color32::from_rgb(18, 118, 178),
            h3: egui::Color32::from_rgb(28, 138, 78),
            code_bg: egui::Color32::from_rgb(226, 230, 252),
            quote_bg: egui::Color32::from_rgba_unmultiplied(148, 158, 218, 40),
            srt_card: egui::Color32::from_rgba_unmultiplied(210, 215, 245, 180),
            srt_index: egui::Color32::from_rgb(65, 85, 215),
            srt_time: egui::Color32::from_rgb(18, 138, 100),
            green: egui::Color32::from_rgb(18, 155, 58),
            red: egui::Color32::from_rgb(200, 38, 38),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Toolbar button
// ─────────────────────────────────────────────────────────────────────────────

fn tbtn(ui: &mut egui::Ui, label: &str, fill: egui::Color32) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(13.0)
                .color(egui::Color32::from_rgb(215, 220, 245)),
        )
        .fill(fill)
        .rounding(egui::Rounding::same(6.0))
        .min_size(egui::vec2(0.0, 28.0)),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Block renderer
// ─────────────────────────────────────────────────────────────────────────────

fn render_blocks(
    ui: &mut egui::Ui,
    blocks: &[Block],
    t: &Theme,
    font_size: f32,
    find: &str,
) {
    let body = font_size;

    for block in blocks {
        match block {
            Block::H1(text) => {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(text)
                        .size(body * 1.85)
                        .color(t.h1)
                        .strong(),
                );
                ui.add(egui::Separator::default().horizontal().spacing(3.0));
                ui.add_space(4.0);
            }
            Block::H2(text) => {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(text)
                        .size(body * 1.48)
                        .color(t.h2)
                        .strong(),
                );
                ui.add_space(2.0);
            }
            Block::H3(text) => {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(text)
                        .size(body * 1.22)
                        .color(t.h3)
                        .strong(),
                );
            }
            Block::Paragraph(text) => {
                ui.add_space(4.0);
                let highlight = !find.is_empty()
                    && text.to_lowercase().contains(&find.to_lowercase());
                let mut rt = egui::RichText::new(text).size(body).color(t.text);
                if highlight {
                    rt = rt.background_color(egui::Color32::from_rgb(100, 80, 0));
                }
                ui.label(rt);
                ui.add_space(2.0);
            }
            Block::CodeBlock(text) => {
                ui.add_space(6.0);
                egui::Frame::default()
                    .fill(t.code_bg)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .font(egui::FontId::monospace(body - 1.5))
                                .color(egui::Color32::from_rgb(175, 215, 255)),
                        );
                    });
                ui.add_space(6.0);
            }
            Block::Quote(text) => {
                ui.add_space(4.0);
                egui::Frame::default()
                    .fill(t.quote_bg)
                    .rounding(egui::Rounding {
                        nw: 0.0,
                        sw: 0.0,
                        ne: 6.0,
                        se: 6.0,
                    })
                    .inner_margin(egui::Margin {
                        left: 14.0,
                        right: 8.0,
                        top: 6.0,
                        bottom: 6.0,
                    })
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .size(body)
                                .color(t.text_dim)
                                .italics(),
                        );
                    });
                ui.add_space(4.0);
            }
            Block::ListItem(text) => {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("  •")
                            .color(t.accent)
                            .size(body + 2.0),
                    );
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(text).size(body).color(t.text));
                });
            }
            Block::Hr => {
                ui.add_space(8.0);
                ui.add(egui::Separator::default().horizontal());
                ui.add_space(8.0);
            }
            Block::SrtEntry { index, time, text } => {
                egui::Frame::default()
                    .fill(t.srt_card)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("#{index}"))
                                    .size(11.0)
                                    .color(t.srt_index)
                                    .strong(),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(time)
                                    .size(11.0)
                                    .color(t.srt_time)
                                    .font(egui::FontId::monospace(11.0)),
                            );
                        });
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(text).size(body).color(t.text),
                        );
                    });
                ui.add_space(4.0);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// eframe App
// ─────────────────────────────────────────────────────────────────────────────

impl eframe::App for LiteNote {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t = if self.dark_mode {
            Theme::dark()
        } else {
            Theme::light()
        };

        // ── Visuals ──────────────────────────────────────────────────────────
        let mut vis = if self.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        vis.panel_fill = t.editor_bg;
        vis.window_fill = t.editor_bg;
        vis.extreme_bg_color = t.code_bg;
        vis.selection.bg_fill = t.accent;
        vis.hyperlink_color = t.accent;
        vis.widgets.inactive.rounding = egui::Rounding::same(6.0);
        vis.widgets.hovered.bg_fill = t.btn_hover;
        vis.widgets.active.bg_fill = t.accent;
        ctx.set_visuals(vis);

        // ── Keyboard shortcuts + drag & drop ─────────────────────────────────
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(egui::Key::N) {
                self.new_file();
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::O) {
                self.open_file();
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::S) {
                self.save_file();
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
                self.show_find = !self.show_find;
            }
            // Drag & drop
            if !i.raw.dropped_files.is_empty() {
                if let Some(f) = i.raw.dropped_files.first() {
                    if let Some(p) = &f.path {
                        let path = p.clone();
                        self.load_file(path);
                    }
                }
            }
        });

        // Copy buffer flush
        if let Some(text) = self.copy_buffer.take() {
            ctx.copy_text(text);
        }

        // ── Toolbar ──────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::default()
                    .fill(t.toolbar_bg)
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;

                    if tbtn(ui, "⊕ New", t.btn_bg).clicked() {
                        self.new_file();
                    }
                    if tbtn(ui, "📂 Open", t.btn_bg).clicked() {
                        self.open_file();
                    }

                    let can_save = matches!(
                        self.file_kind,
                        FileKind::PlainText | FileKind::Markdown | FileKind::Srt
                    );
                    ui.add_enabled_ui(can_save, |ui| {
                        if tbtn(ui, "💾 Save", t.btn_bg).clicked() {
                            self.save_file();
                        }
                        if tbtn(ui, "Save As", t.btn_bg).clicked() {
                            self.save_file_as();
                        }
                    });

                    ui.separator();

                    if tbtn(ui, "📋 Copy All", t.btn_bg).clicked() {
                        self.copy_buffer = Some(self.all_text());
                        self.status = "Copied to clipboard!".to_string();
                        self.status_is_error = false;
                    }

                    ui.separator();

                    let find_fill = if self.show_find { t.accent } else { t.btn_bg };
                    if tbtn(ui, "🔍 Find", find_fill).clicked() {
                        self.show_find = !self.show_find;
                    }

                    ui.separator();

                    ui.checkbox(
                        &mut self.word_wrap,
                        egui::RichText::new("Wrap").size(13.0).color(t.text_dim),
                    );

                    ui.add(
                        egui::Slider::new(&mut self.font_size, 10.0..=36.0)
                            .text(
                                egui::RichText::new("Size")
                                    .size(13.0)
                                    .color(t.text_dim),
                            )
                            .fixed_decimals(0),
                    );

                    // Format badge
                    let (badge, badge_bg) = match self.file_kind {
                        FileKind::Markdown  => ("MD",   egui::Color32::from_rgb(50, 70, 160)),
                        FileKind::Docx      => ("DOCX", egui::Color32::from_rgb(40, 100, 60)),
                        FileKind::Srt       => ("SRT",  egui::Color32::from_rgb(120, 60, 20)),
                        FileKind::PlainText => ("TXT",  egui::Color32::from_rgb(50, 55, 80)),
                        FileKind::Binary    => ("BIN",  egui::Color32::from_rgb(80, 30, 30)),
                    };
                    egui::Frame::default()
                        .fill(badge_bg)
                        .rounding(egui::Rounding::same(5.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(badge)
                                    .size(11.5)
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            );
                        });

                    // Dark/Light — right aligned
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let (icon, lbl) = if self.dark_mode {
                                ("☀", "Light")
                            } else {
                                ("🌙", "Dark")
                            };
                            if tbtn(ui, &format!("{icon} {lbl}"), t.btn_bg).clicked() {
                                self.dark_mode = !self.dark_mode;
                            }
                        },
                    );
                });
            });

        // ── Find bar ─────────────────────────────────────────────────────────
        if self.show_find {
            egui::TopBottomPanel::top("find_bar")
                .frame(
                    egui::Frame::default()
                        .fill(t.toolbar_bg)
                        .inner_margin(egui::Margin::symmetric(14.0, 6.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Find:")
                                .size(13.0)
                                .color(t.text_dim),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.find_query)
                                .min_size(egui::vec2(220.0, 24.0)),
                        );
                        let count = if self.find_query.is_empty() {
                            0
                        } else {
                            let q = self.find_query.to_lowercase();
                            let haystack = if self.blocks.is_empty() {
                                self.content.to_lowercase()
                            } else {
                                self.all_text().to_lowercase()
                            };
                            haystack.matches(&q).count()
                        };
                        let col = if count > 0 { t.green } else { t.red };
                        ui.label(
                            egui::RichText::new(format!("{count} match(es)"))
                                .size(12.0)
                                .color(col),
                        );
                        if tbtn(ui, "✕ Close", t.btn_bg).clicked() {
                            self.show_find = false;
                        }
                    });
                });
        }

        // ── Status bar ────────────────────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::default()
                    .fill(t.toolbar_bg)
                    .inner_margin(egui::Margin::symmetric(14.0, 5.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let sc = if self.status_is_error { t.red } else { t.green };
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(12.0)
                            .color(sc),
                    );
                    if let Some(path) = &self.file_path {
                        ui.label(
                            egui::RichText::new(format!("  {}", path.display()))
                                .size(11.0)
                                .color(t.text_dim),
                        );
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("Drop file here")
                                    .size(11.0)
                                    .color(t.text_dim)
                                    .italics(),
                            );
                            ui.add_space(12.0);
                            let char_count = if self.blocks.is_empty() {
                                self.content.chars().count()
                            } else {
                                self.all_text().chars().count()
                            };
                            ui.label(
                                egui::RichText::new(format!("{char_count} chars"))
                                    .size(12.0)
                                    .color(t.text_dim),
                            );
                        },
                    );
                });
            });

        // ── Main content area ─────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(t.editor_bg)
                    .inner_margin(egui::Margin::same(0.0)),
            )
            .show(ctx, |ui| {
                let is_rendered = matches!(
                    self.file_kind,
                    FileKind::Markdown | FileKind::Docx | FileKind::Srt
                ) && !self.blocks.is_empty();

                if is_rendered {
                    // ── Rendered view ────────────────────────────────────────
                    let find_q = if self.show_find {
                        self.find_query.clone()
                    } else {
                        String::new()
                    };

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // Center content with max width
                            let avail = ui.available_width();
                            let max_w = avail.min(820.0);
                            let pad = ((avail - max_w) / 2.0).max(20.0);

                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                ui.add_space(pad);
                                ui.vertical(|ui| {
                                    ui.set_max_width(max_w);
                                    render_blocks(
                                        ui, &self.blocks, &t,
                                        self.font_size, &find_q,
                                    );
                                    ui.add_space(40.0);
                                });
                            });
                        });
                } else {
                    // ── Plain text editor ────────────────────────────────────
                    let te = egui::TextEdit::multiline(&mut self.content)
                        .font(egui::FontId::monospace(self.font_size))
                        .text_color(t.text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .lock_focus(true)
                        .frame(false);

                    let r = egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(10.0);
                            let r = ui.add(te);
                            ui.add_space(10.0);
                            r
                        })
                        .inner;

                    if r.changed() {
                        self.modified = true;
                    }

                    // Empty state
                    if self.content.is_empty() && self.file_path.is_none() {
                        let rect = ui.max_rect();
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Drop a file here\nor click  📂 Open",
                            egui::FontId::proportional(20.0),
                            egui::Color32::from_rgba_unmultiplied(
                                t.text_dim.r(),
                                t.text_dim.g(),
                                t.text_dim.b(),
                                55,
                            ),
                        );
                    }
                }
            });

        // ── Drag hover overlay ────────────────────────────────────────────────
        ctx.input(|i| {
            if !i.raw.hovered_files.is_empty() {
                let screen = ctx.screen_rect();
                let layer =
                    egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_ov"));
                ctx.layer_painter(layer).rect_filled(
                    screen,
                    egui::Rounding::same(0.0),
                    egui::Color32::from_rgba_unmultiplied(60, 90, 240, 35),
                );
                ctx.layer_painter(layer).text(
                    screen.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop to open",
                    egui::FontId::proportional(34.0),
                    egui::Color32::from_rgb(155, 175, 255),
                );
            }
        });

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size([420.0, 300.0])
            .with_title("LiteNote")
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "LiteNote",
        options,
        Box::new(|_cc| Box::new(LiteNote::default())),
    )
}
