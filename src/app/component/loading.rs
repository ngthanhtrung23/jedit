use std::time::Instant;

use super::popup::popup_area;
use ratatui::{
    layout::Rect,
    prelude::Buffer,
    text::Text,
    widgets::{Block, Clear, Padding, Widget},
};

pub struct Loading(Instant);

impl Default for Loading {
    fn default() -> Self {
        Self::new()
    }
}

impl Loading {
    pub fn new() -> Self {
        Loading(Instant::now())
    }

    fn loading_text(&self) -> Text<'_> {
        let elapsed = (self.0.elapsed().as_secs() % 4) as usize;
        Text::from(String::from_iter(
            "Loading".chars().chain(std::iter::repeat_n('.', elapsed)),
        ))
        .left_aligned()
    }
}

impl Widget for &Loading {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let block = Block::bordered().padding(Padding::symmetric(1, 1));
        let area = popup_area(area, 5, 14);
        Clear.render(area, buf);
        let inner_area = block.inner(area);

        block.render(area, buf);
        self.loading_text().render(inner_area, buf);
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use insta::assert_snapshot;

    use crate::app::component::test_render::render_to_string;

    use super::*;

    #[test]
    fn render_test() {
        for i in 0..5 {
            let loading = Loading(Instant::now() - Duration::from_secs(i));
            assert_snapshot!(render_to_string(&loading));
        }
    }
}
