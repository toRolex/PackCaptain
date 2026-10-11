use ratatui::text::Span;

pub struct Screen {
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    pub frames: usize,
}

impl Screen {
    pub fn replay(raw: &[u8], rows: usize, cols: usize) -> Self {
        let mut screen = Self {
            cells: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
            frames: 0,
        };
        // Crossterm ends each draw with Hide. Do not observe a partial draw.
        let end = raw
            .windows(6)
            .rposition(|bytes| bytes == b"\x1b[?25l")
            .map(|index| index + 6)
            .unwrap_or(0);
        let text = std::str::from_utf8(&raw[..end]).expect("completed frame must be UTF-8");
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                assert_eq!(chars.next(), Some('['), "unsupported escape");
                let mut params = String::new();
                let final_byte = loop {
                    let next = chars.next().expect("incomplete CSI in completed frame");
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break next;
                    }
                    params.push(next);
                };
                match (params.as_str(), final_byte) {
                    ("?25", 'l') => screen.frames += 1,
                    ("?25" | "?1049", 'h' | 'l') => {}
                    (_, 'm') => {}
                    ("2", 'J') => screen.cells.iter_mut().for_each(|row| row.fill(' ')),
                    (_, 'H') => {
                        let mut position = params.split(';');
                        screen.row = position.next().unwrap().parse::<usize>().unwrap() - 1;
                        screen.col = position.next().unwrap().parse::<usize>().unwrap() - 1;
                        assert!(position.next().is_none());
                        assert!(
                            screen.row < rows && screen.col < cols,
                            "cursor outside viewport"
                        );
                    }
                    _ => panic!("unsupported CSI {params}{final_byte}"),
                }
            } else {
                assert!(!ch.is_control(), "unsupported terminal control {ch:?}");
                let width = Span::raw(ch.to_string()).width();
                if width == 0 {
                    panic!("combining characters are outside this fixture's terminal dialect");
                }
                assert!(screen.col + width <= cols, "text outside viewport");
                screen.cells[screen.row][screen.col] = ch;
                for offset in 1..width {
                    screen.cells[screen.row][screen.col + offset] = '\0';
                }
                screen.col += width;
            }
        }
        screen
    }

    pub fn text(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.iter().filter(|&&ch| ch != '\0').collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn selected(&self) -> String {
        self.text()
            .lines()
            .find(|line| line.contains("> ["))
            .unwrap_or("")
            .trim()
            .to_owned()
    }
}

#[test]
fn reconstructs_delta_cells_only_after_complete_draw() {
    let first = "\x1b[?1049h\x1b[5;2H> [ ] Skill031 中文\x1b[?25l";
    let delta = "\x1b[5;13H089\x1b[?25l";
    let raw = format!("{first}{delta}");
    assert!(!raw.contains("Skill089"));
    let screen = Screen::replay(raw.as_bytes(), 18, 120);
    assert_eq!(screen.frames, 2);
    assert!(screen.selected().contains("Skill089 中文"));
    for split in first.len()..raw.len() {
        assert!(
            Screen::replay(&raw.as_bytes()[..split], 18, 120)
                .selected()
                .contains("Skill031")
        );
    }
    let cleared = format!("{raw}\x1b[2J\x1b[5;2H> [ ] Skill000\x1b[?25l");
    let screen = Screen::replay(cleared.as_bytes(), 18, 120);
    assert!(screen.selected().contains("Skill000"));
    assert!(!screen.text().contains("Skill089"));
}

#[test]
#[should_panic(expected = "unsupported CSI")]
fn rejects_unmodeled_terminal_commands() {
    Screen::replay(b"\x1b[1A\x1b[?25l", 18, 120);
}
