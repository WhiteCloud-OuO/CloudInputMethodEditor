//! 原样预编辑与原样提交的一致性、无副作用及光标回归。
mod basic;
mod learner;

use crate::Engine;
use crate::engine::tests::{MemoryLogger, MemoryMeter};
use crate::engine::{input_log::MutedLogger, learning::MutedLearner};
use learner::ObservedLearner;
use std::sync::{Arc, Mutex};

fn assert_raw(engine: &mut Engine, text: &str, cursor_bytes: usize) {
    let learned = Arc::new(Mutex::new(Vec::new()));
    let logged = Arc::new(Mutex::new(Vec::new()));
    let metered = Arc::new(Mutex::new(Vec::new()));
    engine.learner = MutedLearner::new(Box::new(ObservedLearner(learned.clone())));
    engine.logger = MutedLogger::new(Box::new(MemoryLogger(logged.clone())));
    engine.meter = Box::new(MemoryMeter(metered.clone()));
    let composition = engine.composition().clone();
    let history = engine.history.text().to_owned();
    let last_query = engine.last_query.borrow().clone();
    let last_rescored = engine.last_rescored.get();
    let displayed = engine.displayed.clone();
    let recent = engine.recent_commits.len();
    let log_sequence = engine.log_sequence;
    for _ in 0..3 {
        let raw = engine.raw_preedit();
        assert_eq!(raw.text, text);
        assert_eq!(raw.cursor_bytes, cursor_bytes);
        assert!(raw.text.is_char_boundary(raw.cursor_bytes));
        assert_eq!(engine.composition(), &composition);
        assert_eq!(engine.history.text(), history);
        assert_eq!(*engine.last_query.borrow(), last_query);
        assert_eq!(engine.last_rescored.get(), last_rescored);
        assert_eq!(engine.displayed, displayed);
        assert_eq!(engine.recent_commits.len(), recent);
        assert_eq!(engine.log_sequence, log_sequence);
        assert!(learned.lock().unwrap().is_empty());
        assert!(logged.lock().unwrap().is_empty());
        assert!(metered.lock().unwrap().is_empty());
    }
    // 回车原样上屏会把手敲的拼音分隔符 `'` 去掉（`xi'an` → `xian`）
    let committed: String = text.chars().filter(|c| *c != '\'').collect();
    assert_eq!(engine.take_raw(), committed);
    assert!(engine.composition().is_empty());
    assert_eq!(
        logged
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| matches!(entry, crate::InputLogEntry::Commit(_)))
            .count(),
        usize::from(!text.is_empty())
    );
    assert_eq!(metered.lock().unwrap().len(), usize::from(!text.is_empty()));
}
