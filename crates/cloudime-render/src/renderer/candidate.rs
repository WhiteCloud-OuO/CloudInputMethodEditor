//! 候选窗口的第一条页面树路径：候选视图 → 页面树行/格节点 → Document。
//!
//! 这条路径不再经过 `Frame`；旧 `Frame` 渲染仍保留作逐像素对照。第一步先落竖排，
//! 横排 / 矩阵随后接入同一套 `CandidateView` 和页面树节点。

use cloud_ui_core::{
    Constraints, Insets, LayoutResult, Node, NodeKind, PageTree, Position, SizeSpec, layout,
};

use super::item::Item;
use super::{Metrics, Renderer, online_lines, record::DrawRecorder, truncate_with};
use crate::document::{Document, Element, HitRect, HitShape, Region, SelfDrawingPanel};
use crate::elements::{RectElement, TextElement};
use crate::error::RenderError;
use crate::frame::{CandidateView, HighlightRect, PreeditStyle, Row, TipSegment};
use crate::layout::Layout;
use crate::shadow::Shadow;

#[derive(Debug, Clone, Copy, Default)]
struct Columns {
    index_width: f32,
    text_width: f32,
    annotation_width: f32,
    badge_width: f32,
    row_height: f32,
}

pub(super) struct CandidateCache {
    base: CandidateView,
    layout: Layout,
    theme: crate::theme::Theme,
    scale: f32,
    shadow: Option<Shadow>,
    document: Document,
    highlight_index: Option<usize>,
    highlight_rects: Vec<HighlightRect>,
}

struct CandidateLayout {
    rects: Vec<Option<cloud_ui_core::Rect>>,
    top: Option<usize>,
    footer: Option<usize>,
    columns: Columns,
    content: (f32, f32),
    highlight_rects: Vec<HighlightRect>,
}

impl Renderer {
    /// 候选视图直接生成 Document；当前先支持竖排，旧 `Frame` 路径保留作对照。
    pub fn render_candidate_view(
        &self,
        view: &CandidateView,
        layout_mode: Layout,
        theme: &crate::theme::Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<crate::renderer::Rendered, RenderError> {
        let base = CandidateView {
            highlight_animation: None,
            ..view.clone()
        };
        let cached = self.candidate_cache.borrow_mut().take();
        if let Some(mut cache) = cached
            && cache.base == base
            && cache.layout == layout_mode
            && cache.theme == *theme
            && (cache.scale - scale).abs() <= f32::EPSILON
            && cache.shadow == shadow.copied()
        {
            super::set_highlight(
                &mut cache.document,
                cache.highlight_index,
                highlight_rect(view, &cache.highlight_rects),
                scale,
            );
            let result = self
                .render_document(&cache.document, &cache.theme, scale, shadow, false)
                .map(|document| {
                    let mut rendered = document.rendered;
                    super::apply_hit_geometry(&mut rendered, &cache.highlight_rects, scale);
                    rendered
                });
            *self.candidate_cache.borrow_mut() = Some(cache);
            return result;
        }
        if view.columns > 0 {
            return self.render_candidate_matrix(view, layout_mode, theme, scale, shadow);
        }
        if layout_mode == Layout::Horizontal {
            return self.render_candidate_horizontal(view, theme, scale, shadow);
        }
        let metrics = Metrics { theme, scale };
        let candidate = self.candidate_layout(view, layout_mode, &metrics)?;
        let margin = shadow.map_or(0.0, |s| metrics.px(s.margin()));
        let mut recorder = DrawRecorder::new(self.text.clone(), scale, (margin, margin));
        recorder.round_rect(
            margin,
            margin,
            candidate.content.0,
            candidate.content.1,
            metrics.corner_radius(),
            theme.colors.background,
        );
        let mut y = margin + metrics.padding();
        self.draw_candidate_top(&mut recorder, view, &metrics, margin, y);
        y += candidate
            .top
            .and_then(|id| candidate_rect(&candidate.rects, id))
            .map_or(0.0, |rect| rect.height);
        self.draw_candidate_rows(&mut recorder, view, &metrics, &candidate, margin, y);
        if candidate.footer.is_some() {
            self.draw_candidate_info(
                &mut recorder,
                view,
                &metrics,
                margin,
                y + candidate.columns.row_height * view.rows.len() as f32 + metrics.row_padding(),
                candidate.content.0,
            );
        }

        let scale = scale.max(f32::EPSILON);
        let panel = SelfDrawingPanel {
            x: 0.0,
            y: 0.0,
            width: candidate.content.0 / scale,
            height: candidate.content.1 / scale,
            clip: true,
            draw: recorder.into_elements(),
            children: Vec::new(),
            regions: Vec::new(),
        };
        let document = Document {
            width: panel.width,
            height: panel.height,
            elements: vec![Element::SelfDrawingPanel(panel)],
            regions: Vec::new(),
            control_names: Vec::new(),
            instances: Default::default(),
        };
        let mut rendered = self
            .render_document(&document, theme, scale, shadow, false)?
            .rendered;
        rendered.regions = candidate
            .highlight_rects
            .iter()
            .enumerate()
            .map(|(index, rect)| {
                Region::new(
                    format!("candidate_{index}"),
                    HitShape::Rect(HitRect::new(
                        rect.left / scale,
                        rect.top / scale,
                        rect.width() / scale,
                        rect.height() / scale,
                    )),
                    false,
                )
            })
            .collect();
        rendered.highlight_rects = candidate.highlight_rects;
        self.remember_candidate(
            view,
            layout_mode,
            theme,
            scale,
            shadow,
            &document,
            &rendered.highlight_rects,
        );
        Ok(rendered)
    }

    #[allow(clippy::too_many_arguments)]
    fn remember_candidate(
        &self,
        view: &CandidateView,
        layout: Layout,
        theme: &crate::theme::Theme,
        scale: f32,
        shadow: Option<&Shadow>,
        document: &Document,
        highlight_rects: &[HighlightRect],
    ) {
        *self.candidate_cache.borrow_mut() = Some(CandidateCache {
            base: CandidateView {
                highlight_animation: None,
                ..view.clone()
            },
            layout,
            theme: theme.clone(),
            scale,
            shadow: shadow.copied(),
            document: document.clone(),
            highlight_index: super::highlight_element_index(document, theme),
            highlight_rects: highlight_rects.to_vec(),
        });
    }

    fn candidate_layout(
        &self,
        view: &CandidateView,
        layout_mode: Layout,
        m: &Metrics,
    ) -> Result<CandidateLayout, RenderError> {
        if layout_mode != Layout::Vertical || view.columns > 0 {
            return Err(RenderError::InvalidSize {
                width: 0,
                height: 0,
            });
        }
        let columns = self.candidate_columns(&view.rows, m);
        let top_size = self.candidate_top_size(view, m);
        let info_height = self.candidate_info_height(view, m);
        let row_width = m.padding() * 2.0
            + columns.index_width
            + m.column_gap()
            + columns.text_width
            + if columns.annotation_width > 0.0 {
                m.column_gap() + columns.annotation_width
            } else {
                0.0
            }
            + if columns.badge_width > 0.0 {
                m.column_gap() + columns.badge_width
            } else {
                0.0
            };
        let footer_width = view
            .footer
            .as_deref()
            .map(|text| self.measure(text, &m.footer_style()).width)
            .unwrap_or(0.0);
        let width = row_width
            .max(top_size.0 + m.padding() * 2.0)
            .max(footer_width);
        let width = width.max(m.px(m.theme.min_width_pixels));
        let height = m.padding() * 2.0
            + top_size.1
            + columns.row_height * view.rows.len() as f32
            + info_height;
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.node_mut(tree.root).width = SizeSpec::Fixed(width);
        tree.node_mut(tree.root).height = SizeSpec::Fixed(height);
        let top = if top_size.1 > 0.0 {
            Some(add_absolute(
                &mut tree,
                m.padding(),
                m.padding(),
                width,
                top_size.1,
            ))
        } else {
            None
        };
        let rows_top = m.padding() + top_size.1;
        let mut rows = Vec::with_capacity(view.rows.len());
        for index in 0..view.rows.len() {
            rows.push(add_absolute(
                &mut tree,
                m.padding(),
                rows_top + columns.row_height * index as f32,
                width,
                columns.row_height,
            ));
        }
        let footer_top = rows_top + columns.row_height * view.rows.len() as f32 + m.row_padding();
        let footer = (info_height > 0.0).then(|| {
            add_absolute(
                &mut tree,
                m.padding(),
                footer_top,
                width - m.padding() * 2.0,
                info_height,
            )
        });
        let mut children = Vec::new();
        if let Some(top) = top {
            children.push(top);
        }
        children.extend(rows.iter().copied());
        if let Some(footer) = footer {
            children.push(footer);
        }
        tree.set_children(tree.root, children);
        let result: LayoutResult =
            layout(&tree, Constraints::loose(), &mut cloud_ui_core::NoopMeasure);
        let highlight_rects = rows
            .iter()
            .map(|id| {
                let rect = candidate_rect(&result.rects, *id).unwrap_or_default();
                HighlightRect::new(
                    m.padding() / 2.0,
                    rect.y,
                    width - m.padding() / 2.0,
                    rect.bottom(),
                )
            })
            .collect();
        Ok(CandidateLayout {
            rects: result.rects,
            top,
            footer,
            columns,
            content: (width, height),
            highlight_rects,
        })
    }

    fn candidate_columns(&self, rows: &[Row], m: &Metrics) -> Columns {
        let mut columns = Columns::default();
        let text_style = m.text_style();
        let index_style = m.index_style(false);
        let annotation_style = m.annotation_style(m.theme.colors.gloss);
        for row in rows {
            let index = self.measure(&row.index, &index_style);
            let text = self.measure(&row.text, &text_style);
            let auxiliary = self.auxiliary_width(row.auxiliary.as_ref(), m);
            let annotation: f32 = row
                .annotation
                .iter()
                .map(|(text, _)| self.measure(text, &annotation_style).width)
                .sum();
            columns.index_width = columns.index_width.max(index.width);
            columns.text_width = columns.text_width.max(text.width + auxiliary);
            columns.annotation_width = columns.annotation_width.max(annotation);
            columns.badge_width = columns
                .badge_width
                .max(self.badge_width(row.badge.as_deref(), m));
            columns.row_height = columns.row_height.max(text.height + m.row_padding() * 2.0);
        }
        columns
    }

    fn candidate_top_size(&self, view: &CandidateView, m: &Metrics) -> (f32, f32) {
        if !view.has_top_line() {
            return (0.0, 0.0);
        }
        let style = m.pinyin_style(m.theme.colors.pinyin);
        let mut width = 0.0;
        if let Some(preedit) = &view.preedit {
            width += self.measure(&preedit.text(), &style).width + m.px(super::CARET_WIDTH);
        }
        if let Some(status) = &view.status {
            if view.preedit.is_some() {
                width += m.px(super::STATUS_GAP);
            }
            width += self.measure(status, &style).width;
        }
        (width, style.line_height + m.row_padding() * 2.0)
    }

    fn candidate_info_height(&self, view: &CandidateView, m: &Metrics) -> f32 {
        let mut height = if view.tip.is_none() && view.footer.is_none() {
            0.0
        } else {
            let mut bottom = m.px(m.theme.translate_font.line_height);
            if let Some(footer) = view.footer.as_deref() {
                bottom = bottom.max(self.measure(footer, &m.footer_style()).height);
            }
            bottom + m.row_padding()
        };
        if let Some(online) = view.online.as_deref() {
            height += m.px(m.theme.translate_font.line_height) * online_lines(online).len() as f32;
            height += m.row_padding();
        }
        height
    }

    fn draw_candidate_top(
        &self,
        render: &mut DrawRecorder,
        view: &CandidateView,
        m: &Metrics,
        left: f32,
        y: f32,
    ) {
        if !view.has_top_line() {
            return;
        }
        let line_height = m.px(m.theme.pinyin_font.line_height);
        let top = y + m.row_padding();
        let mut x = left + m.padding();
        if let Some(preedit) = &view.preedit {
            let mut cursor_x = x;
            for segment in &preedit.segments {
                let style = match segment.style {
                    PreeditStyle::Typed | PreeditStyle::Rest => {
                        m.pinyin_style(m.theme.colors.pinyin)
                    }
                    PreeditStyle::Struck => m.pinyin_style(m.theme.colors.pinyin).struck(),
                };
                cursor_x += render.text(&segment.text, &style, cursor_x, top);
            }
            let measure_style = m.pinyin_style(m.theme.colors.pinyin);
            let caret_x = x + render
                .measure(&preedit.before_cursor(), &measure_style)
                .width;
            render.rect(
                caret_x,
                top,
                m.px(super::CARET_WIDTH),
                line_height,
                m.theme.colors.pinyin_caret,
            );
            x = cursor_x + m.px(super::CARET_WIDTH);
            if view.status.is_some() {
                x += m.px(super::STATUS_GAP);
            }
        }
        if let Some(status) = &view.status {
            render.text(status, &m.annotation_style(m.theme.colors.gloss), x, top);
        }
    }

    fn draw_candidate_rows(
        &self,
        render: &mut DrawRecorder,
        view: &CandidateView,
        m: &Metrics,
        candidate: &CandidateLayout,
        left: f32,
        y: f32,
    ) {
        let columns = candidate.columns;
        let text_x = left + m.padding() + columns.index_width + m.column_gap();
        let annotation_x = text_x + columns.text_width + m.column_gap();
        let badge_right = left + candidate.content.0 - m.padding();
        if let Some(highlight) = highlight_rect(view, &candidate.highlight_rects) {
            render.round_rect(
                left + highlight.left,
                left + highlight.top,
                highlight.width(),
                highlight.height(),
                m.corner_radius() / 2.0,
                m.theme.colors.highlight,
            );
        }
        for (index, row) in view.rows.iter().enumerate() {
            let top = y + columns.row_height * index as f32 + m.row_padding();
            let small_offset = m.small_offset(m.px(m.theme.text_font.line_height));
            render.text(
                &row.index,
                &m.index_style(view.highlighted == Some(index)),
                left + m.padding(),
                top + m.index_offset(m.px(m.theme.text_font.line_height)),
            );
            let color = if view.highlighted == Some(index) {
                m.theme.colors.highlight_text
            } else {
                m.theme.colors.text
            };
            let style = m
                .style(m.theme.text_font, color)
                .with_family(m.theme.text_family.as_ref());
            render.text(&row.text, &style, text_x, top);
            let text_width = render.measure(&row.text, &m.text_style()).width;
            self.draw_auxiliary(
                render,
                m,
                row.auxiliary.as_ref(),
                text_x + text_width + self.space_width(m),
                top,
            );
            let mut x = annotation_x;
            for (segment, tone) in &row.annotation {
                let style = m.annotation_style(m.tone_color(*tone));
                x += render.text(segment, &style, x, top + small_offset);
            }
            let badge_width = self.badge_width(row.badge.as_deref(), m);
            self.draw_badge(
                render,
                m,
                row.badge.as_deref(),
                badge_right - m.px(super::BADGE_GAP) - badge_width,
                top,
            );
        }
    }

    fn draw_candidate_info(
        &self,
        render: &mut DrawRecorder,
        view: &CandidateView,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) {
        let right = left + content_width - m.padding();
        let mut budget = right - left - m.padding() * 2.0;
        if let Some(footer) = view.footer.as_deref() {
            let style = m.footer_style();
            let size = render.measure(footer, &style);
            render.text(footer, &style, right - size.width, y);
            budget -= size.width + m.column_gap();
        }
        if let Some(segments) = view.tip.as_deref() {
            self.draw_candidate_segments(render, segments, m, left + m.padding(), y, budget);
        }
        if let Some(online) = view.online.as_deref() {
            let bottom = self.candidate_bottom_height(view, m);
            let line_height = m.px(m.theme.translate_font.line_height);
            let budget = content_width - m.padding() * 2.0;
            for (index, line) in online_lines(online).into_iter().enumerate() {
                self.draw_candidate_segments(
                    render,
                    &line,
                    m,
                    left + m.padding(),
                    y + bottom + line_height * index as f32,
                    budget,
                );
            }
        }
    }

    fn candidate_bottom_height(&self, view: &CandidateView, m: &Metrics) -> f32 {
        if view.tip.is_none() && view.footer.is_none() {
            return 0.0;
        }
        let mut height = m.px(m.theme.translate_font.line_height);
        if let Some(footer) = view.footer.as_deref() {
            height = height.max(self.measure(footer, &m.footer_style()).height);
        }
        height + m.row_padding()
    }

    fn draw_candidate_segments(
        &self,
        render: &mut DrawRecorder,
        segments: &[TipSegment],
        m: &Metrics,
        x: f32,
        y: f32,
        budget: f32,
    ) {
        let mut x = x;
        let mut budget = budget;
        for segment in segments {
            if budget <= 0.0 {
                break;
            }
            let style = m.translate_style(m.tone_color(segment.tone), segment.italic);
            let (shown, _) = truncate_with(
                |text, style| render.measure(text, style),
                &segment.text,
                &style,
                budget,
            );
            let used = render.text(&shown, &style, x, y);
            x += used;
            budget -= used;
        }
    }
}

struct HorizontalLayout {
    rects: Vec<Option<cloud_ui_core::Rect>>,
    rows: Vec<usize>,
    items: Vec<Item>,
    row_height: f32,
    content: (f32, f32),
    annotation_height: f32,
    highlight_rects: Vec<HighlightRect>,
}

impl Renderer {
    fn render_candidate_horizontal(
        &self,
        view: &CandidateView,
        theme: &crate::theme::Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<crate::renderer::Rendered, RenderError> {
        let m = Metrics { theme, scale };
        let laid = self.candidate_horizontal_layout(view, &m);
        let margin = shadow.map_or(0.0, |s| m.px(s.margin()));
        let mut recorder = DrawRecorder::new(self.text.clone(), scale, (margin, margin));
        recorder.round_rect(
            margin,
            margin,
            laid.content.0,
            laid.content.1,
            m.corner_radius(),
            theme.colors.background,
        );
        let y = margin + m.padding();
        self.draw_candidate_top(&mut recorder, view, &m, margin, y);
        let top = y + self.candidate_top_size(view, &m).1;
        let inset = m.px(super::HIGHLIGHT_INSET);
        if let Some(rect) = highlight_rect(view, &laid.highlight_rects) {
            recorder.round_rect(
                margin + rect.left,
                margin + rect.top,
                rect.width(),
                rect.height(),
                m.corner_radius() / 2.0,
                theme.colors.highlight,
            );
        }
        for (index, (row, item)) in view.rows.iter().zip(&laid.items).enumerate() {
            let rect = candidate_rect(&laid.rects, laid.rows[index]).unwrap_or_default();
            let x = margin + rect.x;
            let text_top = margin + rect.y + m.row_padding();
            recorder.text(
                &row.index,
                &m.index_style(view.highlighted == Some(index)),
                x,
                text_top + m.index_offset(m.px(m.theme.text_font.line_height)),
            );
            let color = if view.highlighted == Some(index) {
                theme.colors.highlight_text
            } else {
                theme.colors.text
            };
            let style = m
                .style(m.theme.text_font, color)
                .with_family(m.theme.text_family.as_ref());
            let text_x = x + item.index_width + m.px(super::INDEX_GAP);
            recorder.text(&row.text, &style, text_x, text_top);
            self.draw_auxiliary(
                &mut recorder,
                &m,
                row.auxiliary.as_ref(),
                text_x + item.text_width + self.space_width(&m),
                text_top,
            );
            let badge_x =
                x + super::horizontal::item_width(item, &m, self.horizontal_badge_gap(&m))
                    - item.badge_width;
            self.draw_badge(&mut recorder, &m, row.badge.as_deref(), badge_x, text_top);
        }
        if let Some(row) = view.highlighted.and_then(|index| view.rows.get(index)) {
            let annotation_top = top + laid.row_height + m.row_padding() / 2.0;
            let mut x = margin + m.padding() + inset;
            for (segment, tone) in &row.annotation {
                let style = m.annotation_style(m.tone_color(*tone));
                x += recorder.text(segment, &style, x, annotation_top);
            }
        }
        let info_top = if laid.annotation_height > 0.0 {
            top + laid.row_height + laid.annotation_height
        } else {
            y + m.row_padding() + laid.row_height
        };
        self.draw_candidate_info(&mut recorder, view, &m, margin, info_top, laid.content.0);

        let scale = scale.max(f32::EPSILON);
        let panel = SelfDrawingPanel {
            x: 0.0,
            y: 0.0,
            width: laid.content.0 / scale,
            height: laid.content.1 / scale,
            clip: true,
            draw: recorder.into_elements(),
            children: Vec::new(),
            regions: Vec::new(),
        };
        let document = Document {
            width: panel.width,
            height: panel.height,
            elements: vec![Element::SelfDrawingPanel(panel)],
            regions: Vec::new(),
            control_names: Vec::new(),
            instances: Default::default(),
        };
        let mut rendered = self
            .render_document(&document, theme, scale, shadow, false)?
            .rendered;
        rendered.regions = laid
            .highlight_rects
            .iter()
            .enumerate()
            .map(|(index, rect)| {
                Region::new(
                    format!("candidate_{index}"),
                    HitShape::Rect(HitRect::new(
                        rect.left / scale,
                        rect.top / scale,
                        rect.width() / scale,
                        rect.height() / scale,
                    )),
                    false,
                )
            })
            .collect();
        rendered.highlight_rects = laid.highlight_rects;
        self.remember_candidate(
            view,
            Layout::Horizontal,
            theme,
            scale,
            shadow,
            &document,
            &rendered.highlight_rects,
        );
        Ok(rendered)
    }

    fn candidate_horizontal_layout(&self, view: &CandidateView, m: &Metrics) -> HorizontalLayout {
        let (items, row_height) = self.items(&view.rows, m);
        let badge_gap = self.horizontal_badge_gap(m);
        let inset = m.px(super::HIGHLIGHT_INSET);
        let mut width = items
            .iter()
            .map(|item| super::horizontal::item_width(item, m, badge_gap))
            .sum::<f32>()
            + m.column_gap() * items.len().saturating_sub(1) as f32
            + inset * 2.0
            + m.padding() * 2.0;
        let top_size = self.candidate_top_size(view, m);
        if let Some(footer) = view.footer.as_deref() {
            width = width.max(m.column_gap() + self.measure(footer, &m.footer_style()).width);
        }
        width = width.max(m.px(m.theme.min_width_pixels));
        let annotation_height = view
            .highlighted
            .and_then(|index| view.rows.get(index))
            .filter(|row| !row.annotation.is_empty())
            .map_or(0.0, |_| {
                m.px(m.theme.annotation_font.line_height) + m.row_padding()
            });
        let height = m.padding() * 2.0
            + top_size.1
            + row_height
            + annotation_height
            + self.candidate_info_height(view, m);
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.node_mut(tree.root).width = SizeSpec::Fixed(width);
        tree.node_mut(tree.root).height = SizeSpec::Fixed(height);
        let top = (top_size.1 > 0.0)
            .then(|| add_absolute(&mut tree, m.padding(), m.padding(), width, top_size.1));
        let rows_top = m.padding() + top_size.1;
        let mut x = m.padding() + inset;
        let mut rows = Vec::with_capacity(items.len());
        let mut highlight_rects = Vec::with_capacity(items.len());
        for item in &items {
            let item_width = super::horizontal::item_width(item, m, badge_gap);
            rows.push(add_absolute(&mut tree, x, rows_top, item_width, row_height));
            highlight_rects.push(HighlightRect::new(
                x - inset,
                rows_top,
                x + item_width + inset,
                rows_top + row_height,
            ));
            x += item_width + m.column_gap();
        }
        let footer_top = rows_top + row_height + annotation_height + m.row_padding();
        let footer = (self.candidate_info_height(view, m) > 0.0).then(|| {
            add_absolute(
                &mut tree,
                m.padding(),
                footer_top,
                width - m.padding() * 2.0,
                self.candidate_info_height(view, m),
            )
        });
        let mut children = Vec::new();
        if let Some(top) = top {
            children.push(top);
        }
        children.extend(rows.iter().copied());
        if let Some(footer) = footer {
            children.push(footer);
        }
        tree.set_children(tree.root, children);
        let result = layout(&tree, Constraints::loose(), &mut cloud_ui_core::NoopMeasure);
        HorizontalLayout {
            rects: result.rects,
            rows,
            items,
            row_height,
            content: (width, height),
            annotation_height,
            highlight_rects,
        }
    }
}

impl Renderer {
    fn render_candidate_matrix(
        &self,
        view: &CandidateView,
        layout_mode: Layout,
        theme: &crate::theme::Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<crate::renderer::Rendered, RenderError> {
        let m = Metrics { theme, scale };
        let cells = self.matrix_cells(view, &m, layout_mode);
        let grid_rows = view.rows.len().div_ceil(cells.columns.max(1));
        let inset = m.px(super::HIGHLIGHT_INSET);
        let top_size = self.candidate_top_size(view, &m);
        let info_height = self.candidate_info_height(view, &m);
        let width = cells.width(m.column_gap()) + inset * 2.0 + m.padding() * 2.0;
        let height =
            m.padding() * 2.0 + top_size.1 + cells.row_height * grid_rows as f32 + info_height;
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.node_mut(tree.root).width = SizeSpec::Fixed(width);
        tree.node_mut(tree.root).height = SizeSpec::Fixed(height);
        let top = (top_size.1 > 0.0)
            .then(|| add_absolute(&mut tree, m.padding(), m.padding(), width, top_size.1));
        let rows_top = m.padding() + top_size.1;
        let mut rows = Vec::with_capacity(view.rows.len());
        let mut highlight_rects = Vec::with_capacity(view.rows.len());
        for index in 0..view.rows.len() {
            let column = index % cells.columns.max(1);
            let x = m.padding() + inset + cells.offset(column, m.column_gap());
            let y = rows_top + cells.row_height * (index / cells.columns.max(1)) as f32;
            rows.push(add_absolute(
                &mut tree,
                x,
                y,
                cells.column_widths[column],
                cells.row_height,
            ));
            highlight_rects.push(HighlightRect::new(
                x - inset,
                y,
                x + cells.column_widths[column] + inset,
                y + cells.row_height,
            ));
        }
        let footer_top = rows_top + cells.row_height * grid_rows as f32 + m.row_padding();
        let footer = (info_height > 0.0).then(|| {
            add_absolute(
                &mut tree,
                m.padding(),
                footer_top,
                width - m.padding() * 2.0,
                info_height,
            )
        });
        let mut children = Vec::new();
        if let Some(top) = top {
            children.push(top);
        }
        children.extend(rows.iter().copied());
        if let Some(footer) = footer {
            children.push(footer);
        }
        tree.set_children(tree.root, children);
        let result = layout(&tree, Constraints::loose(), &mut cloud_ui_core::NoopMeasure);
        let margin = shadow.map_or(0.0, |s| m.px(s.margin()));
        let mut recorder = DrawRecorder::new(self.text.clone(), scale, (margin, margin));
        recorder.round_rect(
            margin,
            margin,
            width,
            height,
            m.corner_radius(),
            theme.colors.background,
        );
        self.draw_candidate_top(&mut recorder, view, &m, margin, margin + m.padding());
        if let Some(rect) = highlight_rect(view, &highlight_rects) {
            recorder.round_rect(
                margin + rect.left,
                margin + rect.top,
                rect.width(),
                rect.height(),
                m.corner_radius() / 2.0,
                theme.colors.highlight,
            );
        }
        let text_height = m.px(m.theme.text_font.line_height);
        for (index, row) in view.rows.iter().enumerate() {
            let rect = candidate_rect(&result.rects, rows[index]).unwrap_or_default();
            let (text, _) = &cells.texts[index];
            if text.is_empty() && row.index.is_empty() {
                continue;
            }
            let x = margin + rect.x;
            let top = margin + rect.y + m.row_padding();
            if !row.index.is_empty() {
                recorder.text(
                    &row.index,
                    &m.index_style(view.highlighted == Some(index)),
                    x,
                    top + m.index_offset(text_height),
                );
            }
            let color = if view.highlighted == Some(index) {
                theme.colors.highlight_text
            } else {
                theme.colors.text
            };
            let style = m
                .style(m.theme.text_font, color)
                .with_family(m.theme.text_family.as_ref());
            let text_x = x + cells.index_width + m.px(super::INDEX_GAP);
            recorder.text(text, &style, text_x, top);
            let shown_width = recorder.measure(text, &m.text_style()).width;
            self.draw_auxiliary(
                &mut recorder,
                &m,
                row.auxiliary.as_ref(),
                text_x + shown_width + self.space_width(&m),
                top,
            );
            let badge_width = self.badge_width(row.badge.as_deref(), &m);
            self.draw_badge(
                &mut recorder,
                &m,
                row.badge.as_deref(),
                x + rect.width - m.px(super::BADGE_GAP) - badge_width,
                top,
            );
        }
        self.draw_candidate_info(
            &mut recorder,
            view,
            &m,
            margin,
            margin + rows_top + cells.row_height * grid_rows as f32 + m.row_padding(),
            width,
        );
        let panel = SelfDrawingPanel {
            x: 0.0,
            y: 0.0,
            width: width / scale.max(f32::EPSILON),
            height: height / scale.max(f32::EPSILON),
            clip: true,
            draw: recorder.into_elements(),
            children: Vec::new(),
            regions: Vec::new(),
        };
        let document = Document {
            width: panel.width,
            height: panel.height,
            elements: vec![Element::SelfDrawingPanel(panel)],
            regions: Vec::new(),
            control_names: Vec::new(),
            instances: Default::default(),
        };
        let mut rendered = self
            .render_document(&document, theme, scale, shadow, false)?
            .rendered;
        rendered.regions = highlight_rects
            .iter()
            .enumerate()
            .map(|(index, rect)| {
                Region::new(
                    format!("candidate_{index}"),
                    HitShape::Rect(HitRect::new(
                        rect.left / scale.max(f32::EPSILON),
                        rect.top / scale.max(f32::EPSILON),
                        rect.width() / scale.max(f32::EPSILON),
                        rect.height() / scale.max(f32::EPSILON),
                    )),
                    false,
                )
            })
            .collect();
        rendered.highlight_rects = highlight_rects;
        self.remember_candidate(
            view,
            layout_mode,
            theme,
            scale,
            shadow,
            &document,
            &rendered.highlight_rects,
        );
        Ok(rendered)
    }
}

fn add_absolute(tree: &mut PageTree, left: f32, top: f32, width: f32, height: f32) -> usize {
    let mut node = Node::new(NodeKind::Control);
    node.position = Position::Absolute;
    node.insets = Insets {
        left: Some(left),
        top: Some(top),
        right: None,
        bottom: None,
    };
    node.width = SizeSpec::Fixed(width);
    node.height = SizeSpec::Fixed(height);
    tree.push(node)
}

fn candidate_rect(rects: &[Option<cloud_ui_core::Rect>], id: usize) -> Option<cloud_ui_core::Rect> {
    rects.get(id).copied().flatten()
}

fn highlight_rect(view: &CandidateView, rects: &[HighlightRect]) -> Option<HighlightRect> {
    let target = *rects.get(view.highlighted?)?;
    let Some(animation) = view.highlight_animation else {
        return Some(target);
    };
    Some(HighlightRect::lerp(
        animation.from,
        target,
        animation.progress,
    ))
}
