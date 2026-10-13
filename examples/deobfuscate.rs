use jsbeautify::{Options, beautify};

fn main() {
    let obfuscated_code = r#"
var _0x5a3b = ["Hello", "World", "Test", "Message"];
(function (_0x4d8f, _0x3c2a) {
    var _0x1b9e = function (_0x2f7d) {
        while (--_0x2f7d) {
            _0x4d8f.push(_0x4d8f.shift());
        }
    };
    _0x1b9e(2);
})(_0x5a3b, 0x192);
function _0xdec(_0x4c3d) {
    return _0x5a3b[_0x4c3d];
}
console.log(_0xdec(0));
console.log(_0xdec(1));
console.log(_0xdec(2));
    "#;

    println!("=== Original Obfuscated Code ===");
    println!("{obfuscated_code}");
    println!();

    println!("=== Beautified Without Deobfuscation ===");
    let options = Options::default();
    match beautify(obfuscated_code, &options) {
        Ok(result) => println!("{result}"),
        Err(e) => eprintln!("Error: {e}"),
    }
    println!();

    println!("=== Beautified With Deobfuscation ===");
    let options_with_deobf = Options {
        deobfuscate: true,
        ..Options::default()
    };
    match beautify(obfuscated_code, &options_with_deobf) {
        Ok(result) => println!("{result}"),
        Err(e) => eprintln!("Error: {e}"),
    }
}
