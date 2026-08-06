use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

const MAX_LOG_ENTRIES: usize = 2000;

static LOG_BUFFER: LazyLock<Mutex<VecDeque<String>>> =
    LazyLock::new(|| Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)));

pub fn push(msg: String) {
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        if buf.len() >= MAX_LOG_ENTRIES {
            buf.pop_front();
        }
        buf.push_back(msg);
    }
}

pub fn drain() -> Vec<String> {
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        buf.drain(..).collect()
    } else {
        Vec::new()
    }
}

pub fn clear() {
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        buf.clear();
    }
}

#[macro_export]
macro_rules! dev_log {
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        println!("{}", msg);
        $crate::log_buffer::push(msg);
    }};
}

#[macro_export]
macro_rules! dev_elog {
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        eprintln!("{}", msg);
        $crate::log_buffer::push(msg);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ensure a clean buffer before each test since LOG_BUFFER is a global static.
    fn setup() {
        clear();
    }

    #[test]
    fn empty_state_drain_returns_empty() {
        setup();
        let result = drain();
        assert!(result.is_empty());
    }

    #[test]
    fn empty_state_clear_is_noop() {
        setup();
        clear();
        assert!(drain().is_empty());
    }

    #[test]
    fn single_push_drain_returns_it() {
        setup();
        push("hello".to_string());
        let result = drain();
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn drain_clears_buffer() {
        setup();
        push("a".to_string());
        push("b".to_string());
        let first = drain();
        assert_eq!(first, vec!["a", "b"]);
        let second = drain();
        assert!(second.is_empty());
    }

    #[test]
    fn multiple_pushes_preserve_order() {
        setup();
        push("first".to_string());
        push("second".to_string());
        push("third".to_string());
        let result = drain();
        assert_eq!(result, vec!["first", "second", "third"]);
    }

    #[test]
    fn clear_after_push_empties_buffer() {
        setup();
        push("x".to_string());
        push("y".to_string());
        clear();
        assert!(drain().is_empty());
    }

    #[test]
    fn push_after_drain_works() {
        setup();
        push("old".to_string());
        drain();
        push("new".to_string());
        let result = drain();
        assert_eq!(result, vec!["new"]);
    }

    #[test]
    fn capacity_evicts_oldest() {
        // NOTE: Uses global static LOG_BUFFER — cannot assert exact count
        // because parallel tests also push. Verify eviction behavior only.
        setup();
        for i in 0..MAX_LOG_ENTRIES {
            push(format!("msg-{i}"));
        }
        push("overflow".to_string());
        let snapshot = drain();
        // After pushing MAX + 1 entries, buffer should be capped at MAX
        assert!(snapshot.len() <= MAX_LOG_ENTRIES);
        // "msg-0" should have been evicted
        assert!(!snapshot.contains(&"msg-0".to_string()));
        // "overflow" should be the last entry
        assert_eq!(snapshot.last().unwrap(), "overflow");
    }

    #[test]
    fn push_empty_string() {
        setup();
        push(String::new());
        let result = drain();
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn push_unicode() {
        setup();
        push("日本語テスト 🚀".to_string());
        let result = drain();
        assert_eq!(result, vec!["日本語テスト 🚀"]);
    }

    #[test]
    fn dev_log_macro_pushes_to_buffer() {
        setup();
        drain(); // clear stale entries from parallel tests
        dev_log!("fmt {} test", 42);
        let result = drain();
        assert!(result.contains(&"fmt 42 test".to_string()));
    }

    #[test]
    fn dev_log_macro_multiple_invocations() {
        setup();
        drain();
        dev_log!("first_ml");
        dev_log!("second_ml {}", "arg");
        dev_log!("third_ml {}", 1 + 2);
        let result = drain();
        assert!(result.contains(&"first_ml".to_string()));
        assert!(result.contains(&"second_ml arg".to_string()));
        assert!(result.contains(&"third_ml 3".to_string()));
    }

    #[test]
    fn dev_elog_macro_pushes_to_buffer() {
        setup();
        drain();
        dev_elog!("error: {}", "boom");
        let result = drain();
        assert!(result.contains(&"error: boom".to_string()));
    }

    #[test]
    fn dev_elog_macro_multiple_invocations() {
        setup();
        drain(); // clear stale entries from parallel tests
        dev_elog!("err1");
        dev_elog!("err2 code={}", 500);
        let result = drain();
        assert!(result.contains(&"err1".to_string()));
        assert!(result.contains(&"err2 code=500".to_string()));
    }

    #[test]
    fn dev_log_and_dev_elog_share_buffer() {
        setup();
        dev_log!("stdout");
        dev_elog!("stderr");
        let result = drain();
        assert_eq!(result, vec!["stdout", "stderr"]);
    }

    #[test]
    fn macros_with_no_args() {
        setup();
        dev_log!("plain message");
        dev_elog!("plain error");
        let result = drain();
        assert_eq!(result, vec!["plain message", "plain error"]);
    }

    #[test]
    fn drain_after_clear_then_push() {
        setup();
        push("a".to_string());
        clear();
        push("b".to_string());
        let result = drain();
        assert_eq!(result, vec!["b"]);
    }

    #[test]
    fn repeated_drain_is_idempotent() {
        setup();
        push("only".to_string());
        let r1 = drain();
        let r2 = drain();
        let r3 = drain();
        assert_eq!(r1, vec!["only"]);
        assert!(r2.is_empty());
        assert!(r3.is_empty());
    }
}
