use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, Padding, Paragraph, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget,
    },
};

use crate::app::math::Op;

use super::scrollbar::scrollbar;

#[derive(Debug, Default)]
pub(crate) struct SearchState {
    pub query: String,
    pub is_input_mode: bool,
    pub matches: Vec<(usize, usize)>,
    pub current_match: usize,
    pub boundary_message: Option<&'static str>,
}

#[derive(Debug, Default)]
pub struct PreviewState {
    x_offset: u16,
    y_offset: u16,
    pub(crate) search: Option<SearchState>,
}

impl PreviewState {
    pub fn scroll_up(&mut self, n: u16) {
        self.y_offset = Op::Sub(n).exec(self.y_offset);
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.y_offset = Op::Add(n).exec(self.y_offset);
    }

    pub fn scroll_left(&mut self) {
        self.x_offset = Op::Sub(1).exec(self.x_offset);
    }

    pub fn scroll_right(&mut self) {
        self.x_offset = Op::Add(1).exec(self.x_offset);
    }

    pub(crate) fn set_y_offset(&mut self, offset: u16) {
        self.y_offset = offset;
    }

    pub(crate) fn clear_search(&mut self) {
        self.search = None;
    }
}

pub struct Preview {
    content: Option<Content>,
}

impl Preview {
    pub fn new(content: Option<String>) -> Self {
        Self {
            content: content.map(Content::new),
        }
    }

    pub(crate) fn content_text(&self) -> Option<&str> {
        self.content.as_ref().map(|c| c.text.as_str())
    }
}

impl StatefulWidget for &Preview {
    type State = PreviewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let has_search = state.search.is_some();

        let (block_area, search_bar_y) = if has_search && area.height > 2 {
            let mut block_area = area;
            block_area.height -= 1;
            (block_area, Some(area.y + area.height - 1))
        } else {
            (area, None)
        };

        let block = Block::bordered().title("Preview");
        let Some(content) = &self.content else {
            let content_area = block.inner(block_area);
            block.render(block_area, buf);
            let paragraph = Paragraph::new(Line::from("Preview not available").centered());
            let height = paragraph.line_count(content_area.width);
            let vertical =
                Layout::vertical([Constraint::Max(height.try_into().unwrap_or(u16::MAX))])
                    .flex(Flex::Center);
            let [area] = vertical.areas(content_area);
            paragraph.render(area, buf);
            return;
        };

        let scrollbar_area = block.inner(block_area);
        let block = block.padding(Padding::new(0, 2, 0, 2));
        let mut content_area = block.inner(block_area);
        block.render(block_area, buf);

        let line_number_area = content_area;
        let n_digits = content.n_lines.to_string().len().max(3);

        let content_area_shift: u16 = (n_digits + 1).try_into().unwrap_or_default();
        content_area.x += content_area_shift;
        content_area.width -= content_area_shift;

        let y_scroll_size = content
            .n_lines
            .try_into()
            .unwrap_or(u16::MAX)
            .saturating_sub(content_area.height);
        state.y_offset = state.y_offset.min(y_scroll_size);

        let x_scroll_size = content
            .width
            .try_into()
            .unwrap_or(u16::MAX)
            .saturating_sub(content_area.width);
        state.x_offset = state.x_offset.min(x_scroll_size);

        (0..content_area.height)
            .map(|i| state.y_offset + i + 1)
            .take_while(|i| {
                u16::try_from(content.n_lines)
                    .ok()
                    .is_none_or(|n_lines| *i <= n_lines)
            })
            .map(|i| Span::from(number_format(i, n_digits)).style(Style::new().cyan()))
            .collect::<Text<'_>>()
            .render(line_number_area, buf);

        let lines: Text = if let Some(search) = &state.search {
            if !search.matches.is_empty() && !search.query.is_empty() {
                let current_match_style =
                    Style::new().bg(Color::Rgb(200, 150, 0)).fg(Color::Black);
                let other_match_style =
                    Style::new().bg(Color::Rgb(100, 100, 60)).fg(Color::Black);
                let current_line_style = Style::new().bg(Color::Rgb(50, 50, 50));
                let current_match_line = search
                    .matches
                    .get(search.current_match)
                    .map(|&(li, _)| li);
                content
                    .text
                    .lines()
                    .enumerate()
                    .map(|(line_idx, line_str)| {
                        let is_current_line = current_match_line == Some(line_idx);
                        let line_matches: Vec<usize> = search
                            .matches
                            .iter()
                            .filter(|(li, _)| *li == line_idx)
                            .map(|(_, bo)| *bo)
                            .collect();
                        if line_matches.is_empty() {
                            if is_current_line {
                                return Line::from(
                                    Span::raw(line_str).style(current_line_style),
                                );
                            }
                            return Line::from(line_str);
                        }
                        let text_style = if is_current_line {
                            current_line_style
                        } else {
                            Style::new()
                        };
                        let query_len = search.query.len();
                        let mut spans = Vec::new();
                        let mut pos = 0;
                        for &byte_offset in &line_matches {
                            if byte_offset > pos {
                                spans.push(Span::styled(
                                    &line_str[pos..byte_offset],
                                    text_style,
                                ));
                            }
                            let is_current = search.matches.get(search.current_match)
                                == Some(&(line_idx, byte_offset));
                            let style = if is_current {
                                current_match_style
                            } else {
                                other_match_style
                            };
                            let end = (byte_offset + query_len).min(line_str.len());
                            spans.push(Span::styled(&line_str[byte_offset..end], style));
                            pos = end;
                        }
                        if pos < line_str.len() {
                            spans.push(Span::styled(&line_str[pos..], text_style));
                        }
                        Line::from(spans)
                    })
                    .collect::<Text>()
            } else {
                content.text.lines().map(Line::from).collect::<Text>()
            }
        } else {
            content.text.lines().map(Line::from).collect::<Text>()
        };

        Paragraph::new(lines)
            .scroll((state.y_offset, state.x_offset))
            .render(content_area, buf);

        if let Some(search_y) = search_bar_y {
            let search = state.search.as_ref().unwrap();
            let search_x = area.x;
            let width = area.width as usize;
            let left = if search.is_input_mode {
                format!("/{}█", search.query)
            } else {
                format!("/{}", search.query)
            };
            let no_result = !search.is_input_mode && search.matches.is_empty();
            let is_warning = no_result || search.boundary_message.is_some();
            let right = if let Some(msg) = search.boundary_message {
                String::from(msg)
            } else if !search.matches.is_empty() {
                format!("{}/{}", search.current_match + 1, search.matches.len())
            } else if no_result {
                String::from("no result")
            } else {
                String::new()
            };
            let padding = width.saturating_sub(left.len() + right.len());
            let display = format!("{}{:padding$}{}", left, "", right);
            let display = if display.len() > width {
                &display[..width]
            } else {
                &display
            };
            let style = if is_warning {
                Style::new().fg(Color::Rgb(239, 68, 68))
            } else {
                Style::new()
            };
            buf.set_string(search_x, search_y, display, style);
        }

        if y_scroll_size > 0 {
            let mut scrollbar_area = scrollbar_area;
            scrollbar_area.height -= 1;
            let scrollbar = scrollbar(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state =
                ScrollbarState::new((y_scroll_size + 1).into()).position(state.y_offset.into());
            StatefulWidget::render(scrollbar, scrollbar_area, buf, &mut scrollbar_state);
        }

        if x_scroll_size > 0 {
            let mut scrollbar_area = scrollbar_area;
            scrollbar_area.width -= 1;
            let scrollbar = scrollbar(ScrollbarOrientation::HorizontalBottom);
            let mut scrollbar_state =
                ScrollbarState::new((x_scroll_size + 1).into()).position(state.x_offset.into());
            StatefulWidget::render(scrollbar, scrollbar_area, buf, &mut scrollbar_state);
        }
    }
}

fn number_format(index: u16, n_digits: usize) -> String {
    let num = index.to_string();
    (0..n_digits.saturating_sub(num.len()))
        .map(|_| ' ')
        .chain(num.chars())
        .collect()
}

struct Content {
    text: String,
    n_lines: usize,
    width: usize,
}

impl Content {
    fn new(text: String) -> Self {
        let n_lines = text.lines().count();
        let width = text
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_default();

        Self {
            text,
            n_lines,
            width,
        }
    }
}

#[cfg(test)]
mod test {
    use insta::assert_snapshot;

    use crate::{app::component::test_render::stateful_render_to_string, fixtures::SAMPLE_JSON};

    use super::*;

    #[test]
    fn render_short_test() {
        let preview = Preview::new(Some(
            (1..=16).map(|number| number.to_string() + "\n").collect(),
        ));

        assert_snapshot!(stateful_render_to_string(
            &preview,
            &mut PreviewState::default()
        ));

        let preview = Preview::new(Some(
            (1..=20).map(|number| number.to_string() + "\n").collect(),
        ));

        for y_offset in [0, 2, 4] {
            assert_snapshot!(stateful_render_to_string(
                &preview,
                &mut PreviewState {
                    x_offset: 0,
                    y_offset,
                    ..Default::default()
                }
            ));
        }
    }

    #[test]
    fn render_long_line_test() {
        let long_line = (0..74)
            .map(|number| (number % 10).to_string())
            .collect::<String>();
        let longer_line = (0..80)
            .map(|number| (number % 10).to_string())
            .collect::<String>();
        let preview = Preview::new(Some(
            (1..=16)
                .map(|i| {
                    (if i == 10 {
                        longer_line.clone()
                    } else {
                        long_line.clone()
                    }) + "\n"
                })
                .collect(),
        ));

        for x_offset in [0, 2, 4] {
            assert_snapshot!(stateful_render_to_string(
                &preview,
                &mut PreviewState {
                    x_offset,
                    y_offset: 0,
                    ..Default::default()
                }
            ));
        }

        let long_line = (0..75)
            .map(|number| (number % 10).to_string())
            .collect::<String>();

        let preview = Preview::new(Some((1..=16).map(|_| long_line.clone() + "\n").collect()));
        assert_snapshot!(stateful_render_to_string(
            &preview,
            &mut PreviewState::default()
        ));
    }

    #[test]
    fn render_test() {
        let preview = Preview::new(Some(SAMPLE_JSON.to_string()));
        let mut preview_state = PreviewState::default();

        assert_snapshot!(stateful_render_to_string(&preview, &mut preview_state));

        for i in 0..=8 {
            preview_state.scroll_down(2);
            if i % 2 == 0 {
                assert_snapshot!(stateful_render_to_string(&preview, &mut preview_state));
            }
        }

        preview_state.scroll_right();
        assert_snapshot!(stateful_render_to_string(&preview, &mut preview_state));

        preview_state.scroll_up(1);
        assert_snapshot!(stateful_render_to_string(&preview, &mut preview_state));

        preview_state.scroll_left();
        assert_snapshot!(stateful_render_to_string(&preview, &mut preview_state));
    }

    #[test]
    fn render_empty_test() {
        let preview = Preview::new(None);
        assert_snapshot!(stateful_render_to_string(
            &preview,
            &mut PreviewState::default()
        ));
    }
}
