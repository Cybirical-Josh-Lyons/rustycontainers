pub fn _normalize_lines(mut lines: Vec<String>, max: usize) -> Vec<String> {
    if lines.len() > max {
        lines.drain(0..(lines.len() - max));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::_normalize_lines;

    #[test]
    fn normalize_lines_keeps_tail() {
        let lines = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let out = _normalize_lines(lines, 2);
        assert_eq!(out, vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn normalize_lines_noop_when_under_limit() {
        let lines = vec!["a".to_string(), "b".to_string()];
        let out = _normalize_lines(lines.clone(), 5);
        assert_eq!(out, lines);
    }
}
