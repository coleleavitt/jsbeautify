use super::StringArrayRotation;

impl StringArrayRotation {
    pub(super) fn apply_rotation(strings: &mut Vec<String>, count: usize) {
        let len = strings.len();
        if len == 0 {
            return;
        }

        let actual_count = count % len;

        for _ in 0..actual_count {
            if let Some(first) = strings.first().cloned() {
                strings.remove(0);
                strings.push(first);
            }
        }
    }
}
