//! The todo list is a markdown file, because a todo list should outlive its
//! renderer. `~/TODO.md`, standard checkbox syntax; the wallpaper shows it,
//! a click toggles a box, the palette appends. Any editor works on the same
//! file and the watcher picks the change up within two seconds.

use std::path::PathBuf;
use std::sync::Mutex;

struct State {
    /// Every line of the file, verbatim — rewrites must not eat comments.
    lines: Vec<String>,
    /// (line index, done, text) for each checkbox line.
    items: Vec<(usize, bool, String)>,
    mtime: Option<std::time::SystemTime>,
}

static STATE: Mutex<State> = Mutex::new(State { lines: Vec::new(), items: Vec::new(), mtime: None });

const TEMPLATE: &str = "\
# TODO

- [x] 전각 렌더링
- [x] 피드 워커
- [x] git 수집기
- [x] claude 수집기
- [ ] 검색 팔레트 다듬기
- [ ] 드롭존 · 구글드라이브
";

pub fn path() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("TODO.md"))
        .unwrap_or_else(|| "TODO.md".into())
}

fn parse(text: &str) -> (Vec<String>, Vec<(usize, bool, String)>) {
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut items = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        for (prefix, done) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true), ("* [ ] ", false), ("* [x] ", true)] {
            if let Some(rest) = t.strip_prefix(prefix) {
                items.push((i, done, rest.to_string()));
                break;
            }
        }
    }
    (lines, items)
}

/// Reload when the file changed on disk (or seed it on first run).
pub fn refresh() {
    let p = path();
    if !p.exists() {
        let _ = std::fs::write(&p, TEMPLATE);
    }
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let Ok(mut st) = STATE.lock() else { return };
    if st.mtime == mtime && !st.lines.is_empty() {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(&p) {
        let (lines, items) = parse(&text);
        st.lines = lines;
        st.items = items;
        st.mtime = mtime;
    }
}

/// (done, text) rows for the panel, open items first.
pub fn items() -> Vec<(bool, String)> {
    let Ok(st) = STATE.lock() else { return Vec::new() };
    let mut out: Vec<(bool, String)> = st.items.iter().map(|(_, d, t)| (*d, t.clone())).collect();
    out.sort_by_key(|(done, _)| *done);
    out
}

/// Toggle the row as displayed (open-first order) and rewrite the file.
pub fn toggle(display_row: usize) -> bool {
    let Ok(mut st) = STATE.lock() else { return false };
    let mut order: Vec<usize> = (0..st.items.len()).collect();
    order.sort_by_key(|i| st.items[*i].1);
    let Some(&idx) = order.get(display_row) else { return false };
    let (line_no, done, _) = st.items[idx];
    let line = &st.lines[line_no];
    let new_line = if done {
        line.replacen("[x]", "[ ]", 1).replacen("[X]", "[ ]", 1)
    } else {
        line.replacen("[ ]", "[x]", 1)
    };
    st.lines[line_no] = new_line;
    let text = st.lines.join("\n") + "\n";
    if std::fs::write(path(), &text).is_ok() {
        let (lines, items) = parse(&text);
        st.lines = lines;
        st.items = items;
        st.mtime = std::fs::metadata(path()).and_then(|m| m.modified()).ok();
        true
    } else {
        false
    }
}

/// Append a fresh open item (the palette's "+text" action).
pub fn add(text: &str) {
    let p = path();
    refresh();
    let line = format!("- [ ] {}\n", text.trim());
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(line.as_bytes());
    }
    if let Ok(mut st) = STATE.lock() {
        st.mtime = None; // force reload next refresh
    }
    refresh();
    crate::events::post(crate::events::Kind::Ok, "todo added", text.trim());
}
