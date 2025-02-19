use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Stylize,
    text::{Line, Text},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};
use std::io;

fn main() -> Result<()> {
    let branches = read_branches()?;
    let mut terminal = ratatui::init();
    let mut app = App::new(branches)?;
    let app_result = app.run(&mut terminal);
    ratatui::restore();
    app_result
}

#[derive(Debug)]
struct Branch {
    name: String,
}

fn read_branches() -> anyhow::Result<Vec<Branch>> {
    let repo = git2::Repository::open(".")?;
    let branches = repo.branches(None)?;
    let mut out_branches = Vec::new();
    for branch in branches {
        let branch = branch?;
        let n = branch.0.name().unwrap().unwrap();
        out_branches.push(Branch { name: n.to_owned() });
    }
    Ok(out_branches)
}

#[derive(Debug)]
struct App {
    branches: Vec<Branch>,
    curr: usize,
    exit: bool,
}

impl App {
    fn new(branches: Vec<Branch>) -> Result<Self> {
        Ok(Self {
            branches,
            curr: 0,
            exit: false,
        })
    }
    /// runs the application's main loop until the user quits
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
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
            KeyCode::Char('q') => self.exit(),
            KeyCode::Enter => {
                self.switch_branch();
                self.exit();
            }
            KeyCode::Left | KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('h') => {
                self.decrement_counter()
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => {
                self.increment_counter()
            }
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn switch_branch(&mut self) {
        // TODO
    }

    fn increment_counter(&mut self) {
        self.curr += 1;
    }

    fn decrement_counter(&mut self) {
        self.curr -= 1;
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from("Git branch picker");
        let block = Block::new().title(title.centered());
        let lines: Vec<_> = self
            .branches
            .iter()
            .map(|br| Line::from(br.name.clone()))
            .collect();
        let counter_text = Text::from(lines);
        Paragraph::new(counter_text)
            .centered()
            .block(block)
            .render(area, buf)
    }
}
