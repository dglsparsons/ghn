use std::collections::HashMap;

use crate::types::Action;

pub fn is_target_char(ch: char) -> bool {
    matches!(ch, 'm' | 'c' | 'f')
}

fn push_index(indices: &mut Vec<usize>, index: usize, notification_count: usize) {
    if index >= 1 && index <= notification_count {
        indices.push(index);
    }
}

fn push_range(indices: &mut Vec<usize>, start: usize, end: usize, notification_count: usize) {
    let (low, high) = if start <= end { (start, end) } else { (end, start) };
    for index in low..=high {
        push_index(indices, index, notification_count);
    }
}

fn finalize_pending(
    current_number: &mut Option<usize>,
    range_start: &mut Option<usize>,
    indices: &mut Vec<usize>,
    notification_count: usize,
) {
    if let Some(number) = current_number.take() {
        if let Some(start) = range_start.take() {
            push_range(indices, start, number, notification_count);
        } else {
            push_index(indices, number, notification_count);
        }
    } else if let Some(start) = range_start.take() {
        push_index(indices, start, notification_count);
    }
}

pub fn parse_commands(
    input: &str,
    notification_count: usize,
    targets: &HashMap<char, Vec<usize>>,
) -> HashMap<usize, Vec<Action>> {
    let mut result: HashMap<usize, Vec<Action>> = HashMap::new();

    let mut current_number: Option<usize> = None;
    let mut range_start: Option<usize> = None;
    let mut indices: Vec<usize> = Vec::new();
    let mut after_action = false;

    for ch in input.chars() {
        if ch.is_ascii_digit() {
            let digit = ch.to_digit(10).unwrap_or(0) as usize;
            if after_action {
                indices.clear();
                range_start = None;
                current_number = None;
                after_action = false;
            }

            let next = current_number
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit);
            current_number = Some(next);
            continue;
        }

        if ch == '-' {
            if after_action {
                continue;
            }
            if let Some(number) = current_number.take() {
                range_start = Some(number);
            }
            continue;
        }

        if ch == ' ' || ch == ',' {
            finalize_pending(&mut current_number, &mut range_start, &mut indices, notification_count);
            continue;
        }

        if is_target_char(ch) {
            if after_action {
                indices.clear();
                range_start = None;
                current_number = None;
                after_action = false;
            }

            finalize_pending(&mut current_number, &mut range_start, &mut indices, notification_count);
            if let Some(group) = targets.get(&ch) {
                for index in group {
                    if !indices.contains(index) {
                        indices.push(*index);
                    }
                }
            }
            continue;
        }

        if let Some(action) = Action::from_char(ch) {
            finalize_pending(&mut current_number, &mut range_start, &mut indices, notification_count);

            if !indices.is_empty() {
                for index in &indices {
                    result.entry(*index).or_default().push(action);
                }
            }

            // Keep the index list for subsequent actions until new digits appear.
            after_action = true;
            continue;
        }

        current_number = None;
        range_start = None;
        indices.clear();
        after_action = false;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::parse_commands;
    use crate::types::Action;
    use std::collections::HashMap;

    #[test]
    fn parses_single_actions() {
        let targets = HashMap::new();
        let result = parse_commands("1o", 10, &targets);
        assert_eq!(result.get(&1), Some(&vec![Action::Open]));

        let result = parse_commands("3y", 10, &targets);
        assert_eq!(result.get(&3), Some(&vec![Action::Yank]));

        let result = parse_commands("5r", 10, &targets);
        assert_eq!(result.get(&5), Some(&vec![Action::Read]));

        let result = parse_commands("7d", 10, &targets);
        assert_eq!(result.get(&7), Some(&vec![Action::Done]));

        let result = parse_commands("2u", 10, &targets);
        assert_eq!(result.get(&2), Some(&vec![Action::Unsubscribe]));
    }

    #[test]
    fn parses_multi_digit_indices() {
        let targets = HashMap::new();
        let result = parse_commands("11o", 20, &targets);
        assert_eq!(result.get(&11), Some(&vec![Action::Open]));

        let result = parse_commands("123d", 200, &targets);
        assert_eq!(result.get(&123), Some(&vec![Action::Done]));
    }

    #[test]
    fn parses_ranges() {
        let targets = HashMap::new();
        let result = parse_commands("1-3u", 10, &targets);
        assert_eq!(result.get(&1), Some(&vec![Action::Unsubscribe]));
        assert_eq!(result.get(&2), Some(&vec![Action::Unsubscribe]));
        assert_eq!(result.get(&3), Some(&vec![Action::Unsubscribe]));
    }

    #[test]
    fn parses_reverse_ranges() {
        let targets = HashMap::new();
        let result = parse_commands("3-1u", 10, &targets);
        assert_eq!(result.get(&1), Some(&vec![Action::Unsubscribe]));
        assert_eq!(result.get(&2), Some(&vec![Action::Unsubscribe]));
        assert_eq!(result.get(&3), Some(&vec![Action::Unsubscribe]));
    }

    #[test]
    fn parses_lists_with_separators_and_multiple_actions() {
        let targets = HashMap::new();
        let result = parse_commands("1, 2 3 u y", 10, &targets);
        let expected = vec![Action::Unsubscribe, Action::Yank];
        assert_eq!(result.get(&1), Some(&expected));
        assert_eq!(result.get(&2), Some(&expected));
        assert_eq!(result.get(&3), Some(&expected));
    }

    #[test]
    fn parses_multiple_actions_for_same_index() {
        let targets = HashMap::new();
        let result = parse_commands("1o1r1y", 10, &targets);
        assert_eq!(
            result.get(&1),
            Some(&vec![Action::Open, Action::Read, Action::Yank])
        );
    }

    #[test]
    fn ignores_out_of_range_indices() {
        let targets = HashMap::new();
        let result = parse_commands("99o1r", 5, &targets);
        assert_eq!(result.get(&1), Some(&vec![Action::Read]));
        assert!(!result.contains_key(&99));
    }

    #[test]
    fn resets_on_invalid_chars() {
        let targets = HashMap::new();
        let result = parse_commands("1o x 2r", 10, &targets);
        assert_eq!(result.get(&1), Some(&vec![Action::Open]));
        assert_eq!(result.get(&2), Some(&vec![Action::Read]));
    }

    #[test]
    fn supports_repeated_actions_after_single_index() {
        let targets = HashMap::new();
        let result = parse_commands("11oooyd", 20, &targets);
        assert_eq!(
            result.get(&11),
            Some(&vec![
                Action::Open,
                Action::Open,
                Action::Open,
                Action::Yank,
                Action::Done
            ])
        );
    }

    #[test]
    fn parses_status_targets() {
        let mut targets = HashMap::new();
        targets.insert('m', vec![2, 4]);
        let result = parse_commands("md", 10, &targets);
        assert_eq!(result.get(&2), Some(&vec![Action::Done]));
        assert_eq!(result.get(&4), Some(&vec![Action::Done]));
    }
}
