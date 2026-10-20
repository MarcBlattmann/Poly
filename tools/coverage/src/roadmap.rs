//! Parses `docs/ROADMAP.md`: `## Phase N: Title` headings followed by
//! `- [ ]` / `- [x]` milestone lines. Everything else is prose and ignored.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub done: bool,
    /// Plain text with Markdown emphasis and code markers removed.
    pub text: String,
    /// A headline milestone (`**Milestone...**`).
    pub milestone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub number: u32,
    pub title: String,
    pub items: Vec<Item>,
}

impl Phase {
    pub fn done(&self) -> usize {
        self.items.iter().filter(|i| i.done).count()
    }

    pub fn is_complete(&self) -> bool {
        !self.items.is_empty() && self.done() == self.items.len()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roadmap {
    pub phases: Vec<Phase>,
}

impl Roadmap {
    pub fn parse(text: &str) -> Result<Roadmap, String> {
        let mut phases: Vec<Phase> = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if let Some(rest) = line.strip_prefix("## Phase ") {
                let (number, title) = rest
                    .split_once(':')
                    .ok_or_else(|| format!("ROADMAP.md:{}: expected `## Phase N: Title`", n + 1))?;
                let number =
                    number.trim().parse().map_err(|_| format!("ROADMAP.md:{}: bad phase number {number:?}", n + 1))?;
                phases.push(Phase { number, title: title.trim().to_string(), items: Vec::new() });
                continue;
            }
            let trimmed = line.trim_start();
            let (done, rest) = if let Some(r) = trimmed.strip_prefix("- [x] ").or(trimmed.strip_prefix("- [X] ")) {
                (true, r)
            } else if let Some(r) = trimmed.strip_prefix("- [ ] ") {
                (false, r)
            } else {
                continue;
            };
            let phase = phases.last_mut().ok_or_else(|| format!("ROADMAP.md:{}: milestone before any phase", n + 1))?;
            phase.items.push(Item { done, text: plain(rest), milestone: rest.contains("**Milestone") });
        }
        for w in phases.windows(2) {
            if w[1].number != w[0].number + 1 {
                return Err(format!("ROADMAP.md: phase {} follows phase {}", w[1].number, w[0].number));
            }
        }
        Ok(Roadmap { phases })
    }

    pub fn done(&self) -> usize {
        self.phases.iter().map(Phase::done).sum()
    }

    pub fn total(&self) -> usize {
        self.phases.iter().map(|p| p.items.len()).sum()
    }

    /// The first phase that is not complete.
    pub fn current(&self) -> Option<&Phase> {
        self.phases.iter().find(|p| !p.is_complete())
    }
}

fn plain(s: &str) -> String {
    s.replace("**", "").replace('`', "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Roadmap\n\nintro - [ ] not an item before a phase? no, prose\n\n\
        ## Phase 0: Foundation\n\n- [x] Boots\n- [x] Logs `serial`\n\n\
        ## Phase 1: Kernel core\n\ntext\n\n- [ ] Heap\n- [x] **Milestone:** hello\n";

    #[test]
    fn parses_phases_and_items() {
        let r = Roadmap::parse(SAMPLE).unwrap();
        assert_eq!(r.phases.len(), 2);
        assert_eq!(r.phases[0].title, "Foundation");
        assert_eq!(r.phases[0].items[1].text, "Logs serial");
        assert!(r.phases[0].is_complete());
        assert!(r.phases[1].items[1].milestone);
        assert_eq!((r.done(), r.total()), (3, 4));
        assert_eq!(r.current().unwrap().number, 1);
    }

    #[test]
    fn rejects_gaps_and_orphans() {
        assert!(Roadmap::parse("## Phase 0: A\n## Phase 2: C\n").is_err());
        assert!(Roadmap::parse("- [ ] orphan\n").is_err());
    }

    #[test]
    fn real_roadmap_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ROADMAP.md");
        let r = Roadmap::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(r.phases.len() > 5);
        assert_eq!(r.phases[0].number, 0);
    }
}
