#[cfg(test)]
use super::*;

#[test]
fn test_slot_to_name_single_char() {
    let alphabet = BunAlphabet::default_alphabet();

    // Slot 0 -> 'a' (first char of default HEAD)
    assert_eq!(alphabet.slot_to_name(0), "a");
    // Slot 1 -> 'b'
    assert_eq!(alphabet.slot_to_name(1), "b");
    // Slot 25 -> 'z'
    assert_eq!(alphabet.slot_to_name(25), "z");
    // Slot 26 -> 'A'
    assert_eq!(alphabet.slot_to_name(26), "A");
    // Slot 52 -> '_'
    assert_eq!(alphabet.slot_to_name(52), "_");
    // Slot 53 -> '$'
    assert_eq!(alphabet.slot_to_name(53), "$");
}

#[test]
fn test_slot_to_name_two_char() {
    let alphabet = BunAlphabet::default_alphabet();

    // Slot 54 -> 'aa' (first char from HEAD[0], second from TAIL[0])
    assert_eq!(alphabet.slot_to_name(54), "aa");
    // Slot 55 -> 'ba'
    assert_eq!(alphabet.slot_to_name(55), "ba");
}

#[test]
fn test_name_to_slot_single_char() {
    let alphabet = BunAlphabet::default_alphabet();

    assert_eq!(alphabet.name_to_slot("a"), Some(0));
    assert_eq!(alphabet.name_to_slot("b"), Some(1));
    assert_eq!(alphabet.name_to_slot("z"), Some(25));
    assert_eq!(alphabet.name_to_slot("A"), Some(26));
    assert_eq!(alphabet.name_to_slot("_"), Some(52));
    assert_eq!(alphabet.name_to_slot("$"), Some(53));
}

#[test]
fn test_name_to_slot_two_char() {
    let alphabet = BunAlphabet::default_alphabet();

    assert_eq!(alphabet.name_to_slot("aa"), Some(54));
    assert_eq!(alphabet.name_to_slot("ba"), Some(55));
}

#[test]
fn test_roundtrip() {
    let alphabet = BunAlphabet::default_alphabet();

    for slot in 0..1000 {
        let name = alphabet.slot_to_name(slot);
        let recovered = alphabet.name_to_slot(&name);
        assert_eq!(recovered, Some(slot), "Roundtrip failed for slot {slot} -> {name}");
    }
}

#[test]
fn test_custom_alphabet() {
    // Simulate extracted alphabet where 'q' is most frequent
    let head = "qKzYOAwjHJMXuPWfaDZsvGLkbTVgyxEShNRCdnpmIt$iBreFcUQol".to_string();
    let tail = "f68o1ns7q4drKte53_A9$zYO0y2pwjHDMiPaJXkRWvZThNmGLbclESIuVxCgBFQU".to_string();
    let alphabet = BunAlphabet::new(head, tail);

    // Slot 0 should now be 'q'
    assert_eq!(alphabet.slot_to_name(0), "q");
    // Slot 1 should be 'K'
    assert_eq!(alphabet.slot_to_name(1), "K");

    // And reverse
    assert_eq!(alphabet.name_to_slot("q"), Some(0));
    assert_eq!(alphabet.name_to_slot("K"), Some(1));
}

#[test]
fn test_extract_alphabet() {
    let source = r"
            var q = 1;
            var K = 2;
            var z = 3;
            var q6 = 4;
            var K8 = 5;
        ";

    let alphabet = extract_alphabet_from_source(source);

    // 'q' should be early in the alphabet (high frequency)
    let q_pos = alphabet.head.find('q');
    let k_pos = alphabet.head.find('K');
    let z_pos = alphabet.head.find('z');

    assert!(q_pos.is_some());
    assert!(k_pos.is_some());
    assert!(z_pos.is_some());

    // All should be in first few positions
    assert!(q_pos.unwrap() < 10);
    assert!(k_pos.unwrap() < 10);
    assert!(z_pos.unwrap() < 10);
}
