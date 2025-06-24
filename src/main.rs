use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use git2::BranchType;
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{palette::tailwind, Color, Modifier, Style, Stylize},
    text::Text,
    widgets::{
        Block, BorderType, Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, TableState,
    },
    DefaultTerminal, Frame,
};

const ITEM_HEIGHT: usize = 1;
const PALETTES: [tailwind::Palette; 4] = [
    tailwind::BLUE,
    tailwind::EMERALD,
    tailwind::INDIGO,
    tailwind::RED,
];

fn main() -> Result<()> {
    let branches = read_branches()?;
    let mut terminal = ratatui::init();
    let mut app = App::new(branches)?;
    app.run(&mut terminal)?;
    ratatui::restore();
    if app.checkout_on_exit {
        let Some(i) = app.state.selected() else {
            return Ok(());
        };
        let branch_name = &app.repo.branches[i].name;
        let status = std::process::Command::new("git")
            .args(["checkout", branch_name])
            .spawn()?
            .wait()?;
        if !status.success() {
            anyhow::bail!("git checkout failed, status was {status}");
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Branch {
    name: String,
    last_commit: Option<Commit>,
}

#[derive(Debug)]
struct Commit {
    msg: String,
    time: String,
}

impl Branch {
    fn ref_array(&self) -> [String; 3] {
        let msg = self
            .last_commit
            .as_ref()
            .map(|c| c.msg.clone())
            .unwrap_or_default();
        let time = self
            .last_commit
            .as_ref()
            .map(|c| c.time.clone())
            .unwrap_or_default();
        [self.name.clone(), msg, time]
    }
}

#[derive(Debug)]
struct Repo {
    branches: Vec<Branch>,
    root: String,
}

const TIME_PRINTER: jiff::fmt::friendly::SpanPrinter = jiff::fmt::friendly::SpanPrinter::new()
    .spacing(jiff::fmt::friendly::Spacing::BetweenUnitsAndDesignators)
    .comma_after_designator(true)
    .designator(jiff::fmt::friendly::Designator::Verbose);

fn human_friendly_time_since(t: git2::Time) -> Result<String> {
    let committed_at =
        jiff::Timestamp::from_second(t.seconds() + ((t.offset_minutes() as i64) * 60))?;
    let committed_at = committed_at.in_tz("UTC")?.datetime();
    let now = jiff::Zoned::now().datetime();
    let since_commit = (now - committed_at).round(
        jiff::SpanRound::new()
            .smallest(jiff::Unit::Minute)
            .days_are_24_hours(),
    )?;

    Ok(TIME_PRINTER.span_to_string(&-since_commit))
}

fn read_branches() -> anyhow::Result<Repo> {
    let repo = git2::Repository::open_from_env()?;
    let branches = repo.branches(None)?;
    let mut out_branches = Vec::new();
    for branch in branches {
        let (branch, branch_type) = branch?;
        if branch_type == BranchType::Remote {
            continue;
        }
        let name = branch.name()?.unwrap().to_owned();
        let git_ref = branch.get();
        let git_commit = git_ref.peel_to_commit().ok();
        let last_commit = git_commit.map(|c| {
            let human_friendly = human_friendly_time_since(c.time()).unwrap();
            let msg = c.message().unwrap_or("<empty>").to_owned();
            let msg = msg.lines().next().unwrap().to_owned();
            (
                Commit {
                    time: human_friendly,
                    msg,
                },
                c.time(),
            )
        });
        out_branches.push((
            last_commit.as_ref().map(|lc| lc.1),
            Branch {
                name,
                last_commit: last_commit.map(|lc| lc.0),
            },
        ));
    }
    out_branches.sort_by(|x, y| x.0.cmp(&y.0).reverse());
    let out_branches = out_branches.into_iter().map(|x| x.1).collect();

    let root = repo.path().parent().unwrap().display().to_string();
    let home = std::env::var("HOME");
    let root = if let Ok(home) = home {
        if let Some(relative_to_homedir) = root.strip_prefix(&home) {
            format!("~{relative_to_homedir}")
        } else {
            root
        }
    } else {
        root
    };
    Ok(Repo {
        branches: out_branches,
        root,
    })
}

#[derive(Debug)]
struct App {
    repo: Repo,
    exit: bool,
    state: TableState,
    scroll_state: ScrollbarState,
    colors: TableColors,
    longest_item_lens: (u16, u16, u16),
    color_index: usize,
    /// If true, run the git checkout command when the TUI exits.
    checkout_on_exit: bool,
}

#[derive(Debug)]
struct TableColors {
    buffer_bg: Color,
    header_bg: Color,
    header_fg: Color,
    row_fg: Color,
    selected_row_style_fg: Color,
    selected_column_style_fg: Color,
    selected_cell_style_fg: Color,
    normal_row_color: Color,
    footer_border_color: Color,
}

impl TableColors {
    const fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer_bg: tailwind::SLATE.c950,
            header_bg: color.c900,
            header_fg: tailwind::SLATE.c200,
            row_fg: tailwind::SLATE.c200,
            selected_row_style_fg: color.c400,
            selected_column_style_fg: color.c400,
            selected_cell_style_fg: color.c600,
            normal_row_color: tailwind::SLATE.c950,
            footer_border_color: color.c400,
        }
    }
}

impl App {
    fn new(repo: Repo) -> Result<Self> {
        Ok(Self {
            exit: false,
            state: TableState::default().with_selected(0),
            scroll_state: ScrollbarState::new((repo.branches.len() - 1) * ITEM_HEIGHT),
            colors: TableColors::new(&PALETTES[1]),
            color_index: 1,
            longest_item_lens: constraint_len_calculator(&repo.branches),
            repo,
            checkout_on_exit: false,
        })
    }

    fn set_colors(&mut self) {
        self.colors = TableColors::new(&PALETTES[self.color_index]);
    }

    /// runs the application's main loop until the user quits
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let vertical = &Layout::vertical([Constraint::Min(5), Constraint::Length(4)]);
        let rects = vertical.split(frame.area());

        self.set_colors();

        self.render_table(frame, rects[0]);
        self.render_scrollbar(frame, rects[0]);
        self.render_footer(frame, rects[1]);
    }

    fn handle_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') | KeyCode::Esc => self.exit(),
            KeyCode::Enter => {
                self.switch_branch();
                self.exit();
            }
            KeyCode::Left | KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('h') => {
                self.prev_row()
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => {
                self.next_row()
            }
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn switch_branch(&mut self) {
        self.checkout_on_exit = true;
    }

    fn next_row(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.repo.branches.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    fn prev_row(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.repo.branches.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let header_style = Style::default()
            .fg(self.colors.header_fg)
            .bg(self.colors.header_bg);
        let selected_row_style = Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(self.colors.selected_row_style_fg);
        let selected_col_style = Style::default().fg(self.colors.selected_column_style_fg);
        let selected_cell_style = Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(self.colors.selected_cell_style_fg);

        let header = ["Name", "Last commit msg", "Last commit date"]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .style(header_style)
            .height(1);
        let rows = self.repo.branches.iter().map(|data| {
            let color = self.colors.normal_row_color;
            let item = data.ref_array();
            item.into_iter()
                .map(|content| Cell::from(Text::from(content.to_owned())))
                .collect::<Row>()
                .style(Style::new().fg(self.colors.row_fg).bg(color))
                .height(ITEM_HEIGHT.try_into().unwrap())
        });
        let bar = " > ";
        let t = Table::new(
            rows,
            [
                // + 1 is for padding.
                Constraint::Length(self.longest_item_lens.0 + 1),
                Constraint::Min(self.longest_item_lens.1 + 1),
                Constraint::Min(self.longest_item_lens.2),
            ],
        )
        .header(header)
        .row_highlight_style(selected_row_style)
        .column_highlight_style(selected_col_style)
        .cell_highlight_style(selected_cell_style)
        .highlight_symbol(Text::from(vec![bar.into()]))
        .bg(self.colors.buffer_bg)
        .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(t, area, &mut self.state);
    }

    fn render_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 1,
            }),
            &mut self.scroll_state,
        );
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let info_footer = Paragraph::new(Text::from_iter([
            "Gday".to_owned(),
            format!("Repo: {}", self.repo.root),
        ]))
        .style(
            Style::new()
                .fg(self.colors.row_fg)
                .bg(self.colors.buffer_bg),
        )
        .centered()
        .block(
            Block::bordered()
                .border_type(BorderType::Double)
                .border_style(Style::new().fg(self.colors.footer_border_color)),
        );
        frame.render_widget(info_footer, area);
    }
}

fn constraint_len_calculator(items: &[Branch]) -> (u16, u16, u16) {
    let name_len = items
        .iter()
        .map(|b| b.name.chars().count())
        .max()
        .unwrap_or(0);
    let msg_len = items
        .iter()
        .map(|b| {
            b.last_commit
                .as_ref()
                .map(|c| c.msg.chars().count())
                .unwrap_or_default()
        })
        .max()
        .unwrap_or(0);
    let date_len = items
        .iter()
        .map(|b| {
            b.last_commit
                .as_ref()
                .map(|c| c.time.chars().count())
                .unwrap_or_default()
        })
        .max()
        .unwrap_or(0);

    (name_len as u16, msg_len as u16, date_len as u16)
}
