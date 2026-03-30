use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use tui_lib::devkit::playground;
use tui_lib::render::LayoutRenderable;
use tui_lib::theme;
use tui_lib::widget::queue_list::QueueList;
use tui_lib::widget::range_slider::RangeSlider;
use tui_lib::widget::stepper::Stepper;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Rules,
    Detail,
    Threshold,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Rules => Focus::Detail,
            Focus::Detail => Focus::Threshold,
            Focus::Threshold => Focus::Rules,
        }
    }
}

struct DemoState {
    rules: QueueList,
    detail: Stepper,
    threshold: RangeSlider,
    focus: Focus,
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn render(state: &DemoState, area: Rect, buf: &mut Buffer) {
    if area.height < 7 || area.width == 0 {
        return;
    }

    // Help line at bottom.
    let help = match state.focus {
        Focus::Rules => " Tab next section  j/k navigate  d delete  J/K reorder",
        Focus::Detail => " Tab next section  h/l step through options",
        Focus::Threshold => " Tab next section  h/l adjust slider",
    };
    buf.set_stringn(
        area.x,
        area.y + area.height - 1,
        help,
        area.width as usize,
        Style::default().fg(theme::dim()),
    );

    // Shrink area for content.
    let area = Rect::new(area.x, area.y, area.width, area.height - 1);

    let focus_style = Style::default().fg(theme::focus());
    let dim_style = Style::default().fg(theme::dim());

    // Row 0: " Rules" label.
    let rules_label_style = if state.focus == Focus::Rules {
        focus_style
    } else {
        dim_style
    };
    buf.set_stringn(
        area.x,
        area.y,
        " Rules",
        area.width as usize,
        rules_label_style,
    );

    // Rows 1..(height-5): QueueList.
    let list_height = area.height.saturating_sub(6);
    if list_height > 0 {
        let list_area = Rect::new(area.x, area.y + 1, area.width, list_height);
        state.rules.render(list_area, buf);
    }

    // Row (height-5): " Detail Level" label.
    let detail_y = area.y + 1 + list_height;
    let detail_label_style = if state.focus == Focus::Detail {
        focus_style
    } else {
        dim_style
    };
    buf.set_stringn(
        area.x,
        detail_y,
        " Detail Level",
        area.width as usize,
        detail_label_style,
    );

    // Row (height-4): Stepper.
    let stepper_y = detail_y + 1;
    let stepper_area = Rect::new(area.x + 1, stepper_y, area.width.saturating_sub(1), 1);
    state.detail.render(stepper_area, buf);

    // Row (height-3): " Threshold" label.
    let threshold_y = stepper_y + 1;
    let threshold_label_style = if state.focus == Focus::Threshold {
        focus_style
    } else {
        dim_style
    };
    buf.set_stringn(
        area.x,
        threshold_y,
        " Threshold",
        area.width as usize,
        threshold_label_style,
    );

    // Row (height-2): RangeSlider.
    let slider_y = threshold_y + 1;
    let slider_area = Rect::new(area.x + 1, slider_y, area.width.saturating_sub(1), 1);
    state.threshold.render(slider_area, buf);
}

fn tick(_state: &mut DemoState, _dt: Duration) {}

fn apply(state: &mut DemoState, key: &crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Tab => {
            state.focus = state.focus.next();
        }
        _ => match state.focus {
            Focus::Rules => match key.code {
                KeyCode::Down | KeyCode::Char('j') => state.rules.next(),
                KeyCode::Up | KeyCode::Char('k') => state.rules.prev(),
                KeyCode::Char('d') if !state.rules.is_empty() => {
                    let idx = state.rules.selected();
                    state.rules.remove(idx);
                }
                KeyCode::Char('J') => state.rules.move_down(),
                KeyCode::Char('K') => state.rules.move_up(),
                _ => {}
            },
            Focus::Detail => match key.code {
                KeyCode::Left | KeyCode::Char('h') => state.detail.prev(),
                KeyCode::Right | KeyCode::Char('l') => state.detail.next(),
                _ => {}
            },
            Focus::Threshold => match key.code {
                KeyCode::Left | KeyCode::Char('h') => state.threshold.decrement(),
                KeyCode::Right | KeyCode::Char('l') => state.threshold.increment(),
                _ => {}
            },
        },
    }
}

fn main() -> std::io::Result<()> {
    let state = DemoState {
        rules: QueueList::new(vec![
            "Keep last 10 messages".into(),
            "Preserve tool outputs".into(),
            "Summarize after 5min".into(),
            "Drop system prompts".into(),
        ]),
        detail: Stepper::new(vec![
            "Minimal".into(),
            "Standard".into(),
            "Verbose".into(),
            "Debug".into(),
        ]),
        threshold: RangeSlider::new(0.7).steps(10),
        focus: Focus::Rules,
    };

    playground::run_animated_interactive(
        state,
        "Editable Rules",
        render,
        tick,
        apply,
        Duration::from_millis(100),
    )
}
