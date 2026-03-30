use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::render::LayoutRenderable;
use crate::theme;
use crate::widget::tab_bar::TabBar;

#[derive(Clone)]
pub struct State {
    pub tabs: TabBar,
}

pub fn initial_state() -> State {
    State {
        tabs: TabBar::new(vec![
            "Conversation".into(),
            "Edits".into(),
            "Files".into(),
            "Cost".into(),
            "Timeline".into(),
        ]),
    }
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let tab_area = Rect::new(area.x, area.y, area.width, 1);
    let content_area = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );

    state.tabs.render(tab_area, buf);

    match state.tabs.selected() {
        0 => render_conversation(content_area, buf),
        1 => render_edits(content_area, buf),
        2 => render_files(content_area, buf),
        3 => render_cost(content_area, buf),
        4 => render_timeline(content_area, buf),
        _ => {}
    }
}

fn render_styled_lines(lines: &[(&str, Style)], area: Rect, buf: &mut Buffer) {
    for (i, &(text, style)) in lines.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        buf.set_stringn(area.x, y, text, area.width as usize, style);
    }
}

fn render_conversation(area: Rect, buf: &mut Buffer) {
    let style = Style::default().fg(theme::text());
    let dim = Style::default().fg(theme::dim());
    let user_style = Style::default().fg(theme::focus());
    let assistant_style = Style::default().fg(theme::assistant());

    render_styled_lines(
        &[
            ("  [user] Can you refactor the auth module?", user_style),
            ("", dim),
            (
                "  [assistant] Sure. I'll split it into three files:",
                assistant_style,
            ),
            ("    - auth/login.rs", style),
            ("    - auth/session.rs", style),
            ("    - auth/middleware.rs", style),
            ("", dim),
            ("  [user] Looks good. Also add rate limiting.", user_style),
            ("", dim),
            (
                "  [assistant] Done. Added a token-bucket limiter to",
                assistant_style,
            ),
            ("  middleware.rs with configurable burst size.", style),
        ],
        area,
        buf,
    );
}

fn render_edits(area: Rect, buf: &mut Buffer) {
    let add_style = Style::default().fg(theme::success());
    let del_style = Style::default().fg(theme::error());
    let ctx_style = Style::default().fg(theme::dim());
    let header_style = Style::default().fg(theme::warning());

    render_styled_lines(
        &[
            ("  --- a/src/auth/middleware.rs", header_style),
            ("  +++ b/src/auth/middleware.rs", header_style),
            ("  @@ -12,6 +12,14 @@", ctx_style),
            ("   use crate::session::Session;", ctx_style),
            ("  +use crate::rate_limit::TokenBucket;", add_style),
            ("  +", add_style),
            ("  +pub struct RateLimiter {", add_style),
            ("  +    bucket: TokenBucket,", add_style),
            ("  +}", add_style),
            ("   ", ctx_style),
            ("  -pub fn check_auth(req: &Request) {", del_style),
            (
                "  +pub fn check_auth(req: &Request, limiter: &RateLimiter) {",
                add_style,
            ),
            (
                "       let token = req.header(\"Authorization\");",
                ctx_style,
            ),
        ],
        area,
        buf,
    );
}

fn render_files(area: Rect, buf: &mut Buffer) {
    let dir_style = Style::default().fg(theme::focus());
    let file_style = Style::default().fg(theme::text());
    let dim = Style::default().fg(theme::dim());

    render_styled_lines(
        &[
            ("  src/", dir_style),
            ("    auth/", dir_style),
            ("      login.rs", file_style),
            ("      session.rs", file_style),
            ("      middleware.rs", file_style),
            ("      mod.rs", file_style),
            ("    rate_limit/", dir_style),
            ("      mod.rs", file_style),
            ("      token_bucket.rs", file_style),
            ("    main.rs", dim),
            ("    lib.rs", dim),
        ],
        area,
        buf,
    );
}

fn render_cost(area: Rect, buf: &mut Buffer) {
    let label_style = Style::default().fg(theme::dim());
    let value_style = Style::default().fg(theme::text());
    let total_style = Style::default().fg(theme::warning());

    render_styled_lines(
        &[
            ("  Cost Breakdown", total_style),
            ("  ──────────────────────────────", label_style),
            ("  Input tokens:    12,847    $0.032", value_style),
            ("  Output tokens:    4,291    $0.064", value_style),
            ("  Cache reads:      8,103    $0.002", value_style),
            ("  Cache writes:     3,440    $0.011", value_style),
            ("  ──────────────────────────────", label_style),
            ("  Total:                      $0.109", total_style),
            ("", label_style),
            ("  Model: claude-sonnet-4-20250514", label_style),
            ("  Duration: 14.2s", label_style),
        ],
        area,
        buf,
    );
}

fn render_timeline(area: Rect, buf: &mut Buffer) {
    let time_style = Style::default().fg(theme::dim());
    let event_style = Style::default().fg(theme::text());
    let tool_style = Style::default().fg(theme::focus());
    let result_style = Style::default().fg(theme::success());

    render_styled_lines(
        &[
            ("  00:00.0  User message received", event_style),
            ("  00:00.1  Thinking...", time_style),
            ("  00:02.3  Tool: Read src/auth.rs", tool_style),
            ("  00:02.5  Result: 142 lines", result_style),
            ("  00:04.1  Tool: Write src/auth/login.rs", tool_style),
            ("  00:04.2  Result: created (48 lines)", result_style),
            ("  00:06.0  Tool: Write src/auth/session.rs", tool_style),
            ("  00:06.1  Result: created (35 lines)", result_style),
            ("  00:08.4  Tool: Write src/auth/middleware.rs", tool_style),
            ("  00:08.5  Result: created (59 lines)", result_style),
            ("  00:12.1  Tool: Write src/rate_limit/mod.rs", tool_style),
            ("  00:12.2  Result: created (27 lines)", result_style),
            ("  00:14.2  Response complete", event_style),
        ],
        area,
        buf,
    );
}

pub fn tick(_state: &mut State, _dt: Duration) {}

pub fn apply(state: &mut State, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => state.tabs.next(),
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => state.tabs.prev(),
        _ => {}
    }
}
