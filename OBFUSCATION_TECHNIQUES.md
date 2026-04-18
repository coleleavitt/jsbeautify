# Comprehensive JavaScript Obfuscation Techniques Reference Guide

## Overview
This document catalogs 25+ modern JavaScript obfuscation techniques with obfuscated/deobfuscated examples, detection patterns, and tool coverage.

---

## TECHNIQUE 1: Unicode Escape Sequences

**Obfuscated:**
```javascript
var _0x1a2b = '\u0061\u006c\u0065\u0072\u0074';
eval(_0x1a2b + '("Hello")');
```

**Deobfuscated:**
```javascript
var _0x1a2b = 'alert';
eval(_0x1a2b + '("Hello")');
// Or simply:
alert("Hello");
```

**Tools:** obfuscator.io, javascript-obfuscator, Jscrambler
**Detection Pattern:** `/\\u[0-9a-fA-F]{4}/g` or AST: StringLiteral with Unicode escapes
**Handler:** Replace `\uXXXX` with actual characters; decode string literals

---

## TECHNIQUE 2: Hex Escape Sequences

**Obfuscated:**
```javascript
var _0x2c3d = '\x63\x6f\x6e\x73\x6f\x6c\x65';
_0x2c3d['log']('test');
```

**Deobfuscated:**
```javascript
var _0x2c3d = 'console';
_0x2c3d['log']('test');
// Or:
console.log('test');
```

**Tools:** obfuscator.io, javascript-obfuscator
**Detection Pattern:** `/\\x[0-9a-fA-F]{2}/g` or AST: StringLiteral with hex escapes
**Handler:** Replace `\xHH` with ASCII character; decode string literals

---

## TECHNIQUE 3: String Array Rotation

**Obfuscated:**
```javascript
var _0x3e4f = ['alert', 'log', 'console'];
_0x3e4f = _0x3e4f.slice(0x1).concat(_0x3e4f.slice(0x0, 0x1));
// _0x3e4f is now ['log', 'console', 'alert']
var _0x5a = _0x3e4f[0x2]; // 'alert'
```

**Deobfuscated:**
```javascript
var _0x3e4f = ['alert', 'log', 'console'];
// Reverse the rotation
var _0x5a = _0x3e4f[0]; // 'alert'
```

**Tools:** obfuscator.io, javascript-obfuscator
**Detection Pattern:** Array followed by `.slice().concat()` chain; track array index shifts
**Handler:** Trace array mutations; resolve final index to original position

---

## TECHNIQUE 4: String Array with Decoder Function

**Obfuscated:**
```javascript
var _0x4f5a = ['YWxlcnQ=', 'bG9n', 'Y29uc29sZQ=='];
function _0x1234(i) {
  return atob(_0x4f5a[i]);
}
_0x1234(0); // 'alert'
```

**Deobfuscated:**
```javascript
var _0x4f5a = ['alert', 'log', 'console'];
function _0x1234(i) {
  return _0x4f5a[i];
}
_0x1234(0); // 'alert'
```

**Tools:** obfuscator.io (stringArrayEncoding: 'base64'), javascript-obfuscator
**Detection Pattern:** Array of base64/hex strings + decoder function using `atob()`, `Buffer.from()`, or custom XOR
**Handler:** Identify decoder; apply to all array elements; replace calls with decoded values

---

## TECHNIQUE 5: Control Flow Flattening (Switch-based)

**Obfuscated:**
```javascript
var _0x1 = 0;
while (true) {
  switch (_0x1) {
    case 0:
      console.log('step 1');
      _0x1 = 1;
      break;
    case 1:
      console.log('step 2');
      _0x1 = 2;
      break;
    case 2:
      return;
  }
}
```

**Deobfuscated:**
```javascript
console.log('step 1');
console.log('step 2');
```

**Tools:** obfuscator.io (controlFlowFlattening: true), javascript-obfuscator, Jscrambler
**Detection Pattern:** `while(true)` + `switch` with numeric cases; state variable increments
**Handler:** Build control flow graph; trace state transitions; reconstruct sequential blocks

---

## TECHNIQUE 6: Dead Code Injection

**Obfuscated:**
```javascript
function real() {
  console.log('real code');
}
function _0xdead() {
  var x = Math.random();
  if (x > 1) { // Never true
    console.log('dead code');
  }
}
real();
```

**Deobfuscated:**
```javascript
function real() {
  console.log('real code');
}
real();
```

**Tools:** obfuscator.io (deadCodeInjection: true), javascript-obfuscator
**Detection Pattern:** Unreachable code blocks; impossible conditions (`if (false)`, `if (x > 1)` where x ≤ 1)
**Handler:** Constant folding; remove unreachable branches; dead code elimination

---

## TECHNIQUE 7: Constant Folding / Arithmetic Obfuscation

**Obfuscated:**
```javascript
var _0x1 = 0x5 + 0x3; // 8
var _0x2 = 0x10 - 0x2; // 14
var _0x3 = 0x2 * 0x4; // 8
console.log(_0x1 + _0x2 + _0x3); // 30
```

**Deobfuscated:**
```javascript
var _0x1 = 8;
var _0x2 = 14;
var _0x3 = 8;
console.log(30);
```

**Tools:** obfuscator.io, javascript-obfuscator, Jscrambler
**Detection Pattern:** Binary expressions with numeric literals; constant propagation
**Handler:** Evaluate constant expressions at parse time; replace with computed values

---

## TECHNIQUE 8: Identifier Renaming (Hexadecimal Prefix)

**Obfuscated:**
```javascript
function _0x1a2b3c() {
  var _0x4d5e6f = 'secret';
  var _0x7a8b9c = _0x4d5e6f.length;
  return _0x7a8b9c;
}
```

**Deobfuscated:**
```javascript
function getSecretLength() {
  var secret = 'secret';
  var length = secret.length;
  return length;
}
```

**Tools:** obfuscator.io (identifierNamesGenerator: 'hexadecimal'), javascript-obfuscator
**Detection Pattern:** Identifiers matching `/^_0x[0-9a-f]+$/`; track usage patterns
**Handler:** LLM-based renaming (JSimplifier approach) or heuristic-based (variable type inference)

---

## TECHNIQUE 9: Proxy-based Variable Access

**Obfuscated:**
```javascript
var _0x1 = { 'alert': alert, 'log': console.log };
var _0x2 = new Proxy(_0x1, {
  get: function(target, prop) {
    return target[prop];
  }
});
_0x2['alert']('hello');
```

**Deobfuscated:**
```javascript
alert('hello');
```

**Tools:** Custom obfuscators, Jscrambler (advanced)
**Detection Pattern:** `new Proxy()` with `get` trap; property access via bracket notation
**Handler:** Trace Proxy target; resolve property access; eliminate Proxy wrapper

---

## TECHNIQUE 10: Getter/Setter Obfuscation

**Obfuscated:**
```javascript
var _0x1 = {};
Object.defineProperty(_0x1, 'secret', {
  get: function() {
    return 'hidden_value';
  }
});
console.log(_0x1.secret);
```

**Deobfuscated:**
```javascript
console.log('hidden_value');
```

**Tools:** Jscrambler, custom obfuscators
**Detection Pattern:** `Object.defineProperty()` with `get`/`set` descriptors
**Handler:** Evaluate getter/setter; replace property access with computed value

---

## TECHNIQUE 11: Array Destructuring Tricks

**Obfuscated:**
```javascript
var [_0x1, _0x2, _0x3] = [1, 2, 3];
var [a, , c] = ['x', 'y', 'z']; // Skip middle
console.log(_0x1, _0x2, _0x3, a, c);
```

**Deobfuscated:**
```javascript
var _0x1 = 1, _0x2 = 2, _0x3 = 3;
var a = 'x', c = 'z';
console.log(1, 2, 3, 'x', 'z');
```

**Tools:** Modern obfuscators (ES6+)
**Detection Pattern:** ArrayPattern in VariableDeclarator; RestElement usage
**Handler:** Expand destructuring to individual assignments; resolve skipped elements

---

## TECHNIQUE 12: Default Parameter Tricks (Side Effects)

**Obfuscated:**
```javascript
var _0x1 = 0;
function _0x2(x = (_0x1++, 'default')) {
  return x + _0x1;
}
_0x2(); // 'default1'
```

**Deobfuscated:**
```javascript
function _0x2(x) {
  if (x === undefined) x = 'default';
  return x + 1;
}
_0x2(); // 'default1'
```

**Tools:** Custom obfuscators
**Detection Pattern:** SequenceExpression in default parameter; side effects in defaults
**Handler:** Extract side effects; convert to explicit conditional logic

---

## TECHNIQUE 13: Tagged Template Literals

**Obfuscated:**
```javascript
function _0x1(strings, ...values) {
  return strings[0] + values[0] + strings[1];
}
var _0x2 = 'hello';
var _0x3 = _0x1`prefix_${_0x2}_suffix`;
```

**Deobfuscated:**
```javascript
var _0x2 = 'hello';
var _0x3 = 'prefix_' + _0x2 + '_suffix';
```

**Tools:** Custom obfuscators
**Detection Pattern:** TaggedTemplateExpression; custom tag functions
**Handler:** Evaluate tag function; convert to string concatenation

---

## TECHNIQUE 14: eval() / new Function() String Execution

**Obfuscated:**
```javascript
var _0x1 = 'console.log("executed")';
eval(_0x1);
var _0x2 = new Function('return alert("dynamic")');
_0x2();
```

**Deobfuscated:**
```javascript
console.log("executed");
alert("dynamic");
```

**Tools:** All obfuscators (common wrapper)
**Detection Pattern:** `eval()` or `new Function()` with string arguments
**Handler:** Extract string argument; parse as code; inline result (with safety checks)

---

## TECHNIQUE 15: Regex-based Decoder Tricks

**Obfuscated:**
```javascript
var _0x1 = 'aXbYcZ';
var _0x2 = _0x1.replace(/[a-z]/g, function(m) {
  return String.fromCharCode(m.charCodeAt(0) + 1);
});
// _0x2 = 'bYcZdA'
```

**Deobfuscated:**
```javascript
var _0x2 = 'bYcZdA';
```

**Tools:** Custom obfuscators
**Detection Pattern:** `.replace()` with regex + callback function; character code manipulation
**Handler:** Trace regex matches; evaluate callback; compute result

---

## TECHNIQUE 16: Reduce/Map/Filter Pipelines

**Obfuscated:**
```javascript
var _0x1 = [1, 2, 3, 4, 5];
var _0x2 = _0x1
  .filter(x => x % 2 === 0)
  .map(x => x * 2)
  .reduce((a, b) => a + b, 0);
// _0x2 = 12
```

**Deobfuscated:**
```javascript
var _0x2 = 12;
```

**Tools:** Custom obfuscators
**Detection Pattern:** Chained `.filter()`, `.map()`, `.reduce()` with constant inputs
**Handler:** Constant propagation; evaluate pipeline; replace with result

---

## TECHNIQUE 17: Number System Obfuscation (Hex/Binary/Scientific)

**Obfuscated:**
```javascript
var _0x1 = 0xFF; // 255
var _0x2 = 0b11111111; // 255
var _0x3 = 2.55e2; // 255
console.log(_0x1, _0x2, _0x3);
```

**Deobfuscated:**
```javascript
var _0x1 = 255;
var _0x2 = 255;
var _0x3 = 255;
console.log(255, 255, 255);
```

**Tools:** obfuscator.io, javascript-obfuscator
**Detection Pattern:** Numeric literals in hex (`0x`), binary (`0b`), or scientific notation
**Handler:** Parse and convert to decimal; replace with normalized form

---

## TECHNIQUE 18: Computed Property Keys

**Obfuscated:**
```javascript
var _0x1 = {};
_0x1['con' + 'sole'] = console;
_0x1['lo' + 'g']('test');
// Equivalent to: _0x1.console.log('test')
```

**Deobfuscated:**
```javascript
console.log('test');
```

**Tools:** obfuscator.io (transformObjectKeys: true), javascript-obfuscator
**Detection Pattern:** BinaryExpression (string concatenation) in computed property; constant folding
**Handler:** Evaluate string concatenation; resolve property name; simplify access

---

## TECHNIQUE 19: Comma Expression Junk

**Obfuscated:**
```javascript
var _0x1 = (0, console).log; // Comma operator; evaluates to console
_0x1('test');
var _0x2 = (Math.random(), 'value'); // Evaluates to 'value'
```

**Deobfuscated:**
```javascript
var _0x1 = console.log;
_0x1('test');
var _0x2 = 'value';
```

**Tools:** Custom obfuscators
**Detection Pattern:** SequenceExpression in CallExpression or AssignmentExpression
**Handler:** Extract last element of sequence; discard side effects (or trace if needed)

---

## TECHNIQUE 20: Spread-based Argument Shuffling

**Obfuscated:**
```javascript
function _0x1(a, b, c) {
  return a + b + c;
}
var _0x2 = [1, 2, 3];
_0x1(..._0x2); // 6
```

**Deobfuscated:**
```javascript
function _0x1(a, b, c) {
  return a + b + c;
}
_0x1(1, 2, 3); // 6
```

**Tools:** Modern obfuscators (ES6+)
**Detection Pattern:** SpreadElement in CallExpression; array literal with known values
**Handler:** Expand spread operator; inline array elements as arguments

---

## TECHNIQUE 21: Label-based Control Flow

**Obfuscated:**
```javascript
outer: for (var i = 0; i < 3; i++) {
  for (var j = 0; j < 3; j++) {
    if (i === 1 && j === 1) {
      continue outer;
    }
    console.log(i, j);
  }
}
```

**Deobfuscated:**
```javascript
for (var i = 0; i < 3; i++) {
  for (var j = 0; j < 3; j++) {
    if (i === 1 && j === 1) {
      break; // or restructure logic
    }
    console.log(i, j);
  }
}
```

**Tools:** Custom obfuscators
**Detection Pattern:** LabeledStatement with BreakStatement/ContinueStatement referencing label
**Handler:** Trace label scope; convert to equivalent control flow (may require restructuring)

---

## TECHNIQUE 22: Packer-style Wrappers (Dean Edwards)

**Obfuscated:**
```javascript
eval(function(p,a,c,k,e,r){e=function(c){return c.toString(36)};if(!''.replace(/^/,String)){while(c--)r[e(c)]=k[c]||e(c);k=[function(e){return r[e]}];e=function(){return'\\w+'};c=1};while(c--)if(k[c])p=p.replace(new RegExp('\\b'+e(c)+'\\b','g'),k[c]);return p}('0 1(2){3.4(2)}',5,5,'function|test|x|console|log'.split('|'),0,{}))
```

**Deobfuscated:**
```javascript
function test(x){console.log(x)}
```

**Tools:** Dean Edwards Packer, custom packers
**Detection Pattern:** `eval(function(p,a,c,k,e,r){...})` wrapper; specific parameter names
**Handler:** Extract packed code; apply unpacking algorithm; parse result

---

## TECHNIQUE 23: JJencode (Japanese-style Obfuscation)

**Obfuscated:**
```javascript
$=~[];$={___:++$,$$$$:(![]+"")[$],__$:++$,$_$_:(![]+"")[$],_$_:++$,$_$$:(![]+"")[$],$$_$:++$};$.$_=($.$_=$+"")[$ .$_$]+($._$=$.$_[$.__$])+($.$=($.$+$)[$ .$_$])+(![]+"")[$._$_]+($.__=$.$_[$.$$_$])+($.$=(![]+"")[$._$_])+($._=(![]+"")[$._$_])+$.$_[$.__$]+$.__+$._$+$.$;$.$$=$.$+(![]+"")[$._$_]+$.__+$._+$.$+$.$$;$.$=($.___)[$.$_][$.$_];$.$($.$($.$$+"\""+$.$_$_+(![]+"")[$._$_]+$.$_$$+"\\"+$.__$+$.$$_$+$._$_+$.__+"(\\\""+$._$_$+"\\\")"+"\"")())();
```

**Deobfuscated:**
```javascript
alert("Hello")
```

**Tools:** JJencode, custom encoders
**Detection Pattern:** Heavy use of `$`, `_`, `[]`, `![]`, `+[]`; no readable identifiers
**Handler:** Use dedicated JJencode decoder (e.g., de4js); reverse character mapping

---

## TECHNIQUE 24: AAencode (Alphanumeric Obfuscation)

**Obfuscated:**
```javascript
ﾟωﾟノ= /｀ｍ´)ﾉ ~┻━┻   //*´∇｀*/ ['_'];o=(ﾟｰﾟ)  =_=3; c=(ﾟΘﾟ) =(ﾟｰﾟ)-(ﾟｰﾟ); (ﾟДﾟ) =(ﾟΘﾟ)= (o^_^o)/ (o^_^o);(ﾟДﾟ)={ﾟΘﾟ: '_' ,ﾟωﾟノ : ((ﾟωﾟノ==3) +'_') [ﾟΘﾟ] ,ﾟ__ﾟ : (_++) ,ﾟ_ﾟ: (ﾟДﾟ) [ﾟωﾟノ] [ﾟΘﾟ] ,ﾟಠہಠﾟノ,ﾟｰﾟﾉ :(ﾟДﾟ) [ﾟ__ﾟ] [ﾟ__ﾟ] ,ﾟДﾟﾉ:((_)++) ,ﾟΘﾟﾉ:(_=ﾟΘﾟ==3?"\\\\":"\\\\\\\\":ﾟΘﾟ) ,ﾟｰﾟ*: (_++) ,ﾟｰﾟ|_|_|: ﾟωﾟノ [ﾟΘﾟ] [ﾟΘﾟ] ,ﾟ∩ﾟ|_|_|:(_/ﾟΘﾟ)[ﾟΘﾟ]};(ﾟωﾟノ+ ++ﾟДﾟ)[ﾟωﾟノ+ﾟ__ﾟ]+...
```

**Deobfuscated:**
```javascript
alert("Hello")
```

**Tools:** AAencode, de4js
**Detection Pattern:** Unicode characters (ﾟ, ω, ノ, etc.); non-ASCII identifiers
**Handler:** Use dedicated AAencode decoder; reverse Unicode mapping

---

## TECHNIQUE 25: WebAssembly Blob Obfuscation

**Obfuscated:**
```javascript
var _0x1 = 'AGFzbQEAAAABBwFgAn9/AX8DAgABBwcBBG1haW4AA...'; // base64 WASM
var _0x2 = Uint8Array.from(atob(_0x1), c => c.charCodeAt(0));
var _0x3 = new WebAssembly.Instance(
  new WebAssembly.Module(_0x2)
);
_0x3.exports.main();
```

**Deobfuscated:**
```javascript
// Decode WASM blob
var wasmBinary = Uint8Array.from(atob(_0x1), c => c.charCodeAt(0));
// Disassemble WASM to understand logic
// (requires wasm2wat or similar tool)
```

**Tools:** wasm-obfuscator, custom WASM packers
**Detection Pattern:** `WebAssembly.instantiate()`, `WebAssembly.Module()`, base64 blob + `atob()`
**Handler:** Extract WASM binary; disassemble with wasm2wat; analyze exported functions

---

## TECHNIQUE 26: VM Obfuscation (Opcode-based)

**Obfuscated:**
```javascript
var _0x1 = [
  [0x1, 0x5], // PUSH 5
  [0x2, 0x3], // PUSH 3
  [0x3],      // ADD
  [0x4]       // RETURN
];
var _0x2 = [];
for (var _0x3 = 0; _0x3 < _0x1.length; _0x3++) {
  var _0x4 = _0x1[_0x3];
  switch (_0x4[0]) {
    case 0x1: _0x2.push(_0x4[1]); break;
    case 0x2: _0x2.push(_0x4[1]); break;
    case 0x3: _0x2.push(_0x2.pop() + _0x2.pop()); break;
    case 0x4: return _0x2[0];
  }
}
```

**Deobfuscated:**
```javascript
var result = 5 + 3; // 8
```

**Tools:** Ruam, PISTOL VM, custom VM obfuscators
**Detection Pattern:** Opcode array + switch-based interpreter; stack manipulation
**Handler:** Trace opcode execution; build bytecode disassembler; reconstruct high-level logic

---

## TECHNIQUE 27: Symbol-based Obfuscation

**Obfuscated:**
```javascript
var _0x1 = Symbol('secret');
var _0x2 = {};
_0x2[_0x1] = 'hidden_value';
console.log(_0x2[_0x1]); // 'hidden_value'
console.log(Object.keys(_0x2)); // [] (symbols not enumerable)
```

**Deobfuscated:**
```javascript
var _0x2 = { secret: 'hidden_value' };
console.log(_0x2.secret); // 'hidden_value'
```

**Tools:** Custom obfuscators (ES6+)
**Detection Pattern:** `Symbol()` usage; bracket notation with Symbol; `Object.getOwnPropertySymbols()`
**Handler:** Trace Symbol creation; resolve property access; convert to string keys

---

## TECHNIQUE 28: Opaque Predicates (Conditional Junk)

**Obfuscated:**
```javascript
if (Math.PI > 3 && Math.PI < 4) {
  console.log('real code');
} else {
  console.log('dead code');
}
```

**Deobfuscated:**
```javascript
console.log('real code');
```

**Tools:** Jscrambler, custom obfuscators
**Detection Pattern:** Conditions with known-true/known-false predicates (Math constants, type checks)
**Handler:** Constant folding; evaluate predicate; eliminate dead branch

---

## TECHNIQUE 29: Honey Opcodes (VM Decoys)

**Obfuscated:**
```javascript
var _0x1 = [
  [0x1, 0x5],      // PUSH 5 (real)
  [0xFF, 0xDEAD],  // HONEY_OP (decoy, never executed)
  [0x2, 0x3],      // PUSH 3 (real)
  [0x3],           // ADD (real)
  [0xFE, 0xBEEF]   // HONEY_OP (decoy)
];
// VM skips honey opcodes via conditional checks
```

**Deobfuscated:**
```javascript
var result = 5 + 3; // 8
```

**Tools:** PerimeterX VM, Datadome VM, advanced obfuscators
**Detection Pattern:** Opcode array with unreachable/invalid opcodes; conditional VM dispatch
**Handler:** Trace VM execution; identify reachable opcodes; discard honey opcodes

---

## TECHNIQUE 30: Multi-layer Encryption (Nested Decoders)

**Obfuscated:**
```javascript
var _0x1 = 'aGVsbG8gd29ybGQ='; // base64
var _0x2 = atob(_0x1); // 'hello world'
var _0x3 = _0x2.split('').map(c => 
  String.fromCharCode(c.charCodeAt(0) ^ 0x42)
).join(''); // XOR layer
// _0x3 = '\x27\x2e\x2e\x2e' (encrypted)
```

**Deobfuscated:**
```javascript
var result = 'hello world';
```

**Tools:** Jscrambler, custom obfuscators
**Detection Pattern:** Nested `atob()`, `Buffer.from()`, XOR, or custom decoder chains
**Handler:** Trace decoder chain; apply each layer in reverse; extract final value

---

## TECHNIQUE 31: Computed Function Names

**Obfuscated:**
```javascript
var _0x1 = 'con' + 'sole';
var _0x2 = 'lo' + 'g';
window[_0x1][_0x2]('test');
```

**Deobfuscated:**
```javascript
console.log('test');
```

**Tools:** obfuscator.io, javascript-obfuscator
**Detection Pattern:** BinaryExpression (string concat) in MemberExpression
**Handler:** Constant folding; resolve property chain; simplify access

---

## TECHNIQUE 32: Bitwise Operators for Obfuscation

**Obfuscated:**
```javascript
var _0x1 = 5 << 1; // 10
var _0x2 = 16 >> 2; // 4
var _0x3 = 0xFF ^ 0xAA; // 85
console.log(_0x1 | _0x2 | _0x3); // 15
```

**Deobfuscated:**
```javascript
console.log(15);
```

**Tools:** Custom obfuscators
**Detection Pattern:** Bitwise operators (`<<`, `>>`, `^`, `|`, `&`) with numeric literals
**Handler:** Constant folding; evaluate bitwise operations; replace with result

---

## TECHNIQUE 33: Ternary Operator Chains

**Obfuscated:**
```javascript
var _0x1 = Math.random() > 0.5 ? 'a' : 'b';
var _0x2 = _0x1 === 'a' ? 'x' : _0x1 === 'b' ? 'y' : 'z';
console.log(_0x2);
```

**Deobfuscated:**
```javascript
// Depends on runtime value; cannot fully deobfuscate without execution
// But can simplify structure:
var _0x1 = Math.random() > 0.5 ? 'a' : 'b';
var _0x2 = _0x1 === 'a' ? 'x' : 'y';
console.log(_0x2);
```

**Tools:** Custom obfuscators
**Detection Pattern:** Nested ConditionalExpression; constant branches
**Handler:** Simplify constant branches; trace data flow; eliminate unreachable paths

---

## TECHNIQUE 34: Function Hoisting Tricks

**Obfuscated:**
```javascript
console.log(_0x1()); // Hoisted function call
function _0x1() {
  return 'hoisted';
}
var _0x2 = function() { return 'not hoisted'; };
console.log(_0x2()); // Error if called before declaration
```

**Deobfuscated:**
```javascript
function _0x1() {
  return 'hoisted';
}
console.log(_0x1()); // 'hoisted'
var _0x2 = function() { return 'not hoisted'; };
console.log(_0x2()); // 'not hoisted'
```

**Tools:** Custom obfuscators
**Detection Pattern:** CallExpression before FunctionDeclaration; hoisting analysis
**Handler:** Reorder declarations; trace hoisting scope; resolve call targets

---

## TECHNIQUE 35: Closure-based State Hiding

**Obfuscated:**
```javascript
var _0x1 = (function() {
  var _0x2 = 'secret'; // Captured in closure
  return function() {
    return _0x2;
  };
})();
console.log(_0x1()); // 'secret'
```

**Deobfuscated:**
```javascript
function _0x1() {
  return 'secret';
}
console.log(_0x1()); // 'secret'
```

**Tools:** Custom obfuscators
**Detection Pattern:** IIFE with captured variables; ReturnStatement returning function
**Handler:** Trace closure scope; inline captured values; simplify function

---

## Summary Table

| # | Technique | Complexity | Tools | Detection |
|---|-----------|-----------|-------|-----------|
| 1 | Unicode Escapes | Low | obfuscator.io, js-obfuscator | `/\\u[0-9a-fA-F]{4}/` |
| 2 | Hex Escapes | Low | obfuscator.io, js-obfuscator | `/\\x[0-9a-fA-F]{2}/` |
| 3 | String Array Rotation | Medium | obfuscator.io, js-obfuscator | `.slice().concat()` chains |
| 4 | String Array Decoder | Medium | obfuscator.io | `atob()`, `Buffer.from()` |
| 5 | Control Flow Flattening | High | obfuscator.io, Jscrambler | `while(true)` + `switch` |
| 6 | Dead Code Injection | Low | obfuscator.io, js-obfuscator | Unreachable blocks |
| 7 | Constant Folding | Low | All | Numeric expressions |
| 8 | Identifier Renaming | Medium | obfuscator.io, js-obfuscator | `_0x[0-9a-f]+` pattern |
| 9 | Proxy Access | Medium | Jscrambler, custom | `new Proxy()` |
| 10 | Getter/Setter | Medium | Jscrambler, custom | `Object.defineProperty()` |
| 11 | Array Destructuring | Low | Modern obfuscators | `[a, b, c] = ...` |
| 12 | Default Parameters | Low | Custom | Side effects in defaults |
| 13 | Tagged Templates | Low | Custom | `` tag`...` `` |
| 14 | eval/Function | High | All | `eval()`, `new Function()` |
| 15 | Regex Decoder | Medium | Custom | `.replace()` + callback |
| 16 | Reduce/Map/Filter | Medium | Custom | Chained array methods |
| 17 | Number Systems | Low | obfuscator.io, js-obfuscator | `0x`, `0b`, scientific |
| 18 | Computed Properties | Low | obfuscator.io | String concat in keys |
| 19 | Comma Expressions | Low | Custom | SequenceExpression |
| 20 | Spread Operator | Low | Modern obfuscators | `...array` |
| 21 | Label Control Flow | Medium | Custom | `label:` + `break/continue` |
| 22 | Packer Wrapper | High | Dean Edwards Packer | `eval(function(p,a,c,k...` |
| 23 | JJencode | High | JJencode | `$`, `_`, `[]`, `![]` |
| 24 | AAencode | High | AAencode | Unicode chars (ﾟ, ω, ノ) |
| 25 | WASM Blob | High | wasm-obfuscator | `WebAssembly.instantiate()` |
| 26 | VM Opcodes | Very High | Ruam, PISTOL, custom | Opcode array + switch |
| 27 | Symbol Obfuscation | Medium | Custom (ES6+) | `Symbol()` usage |
| 28 | Opaque Predicates | Medium | Jscrambler, custom | Known-true conditions |
| 29 | Honey Opcodes | Very High | PerimeterX, Datadome | Unreachable opcodes |
| 30 | Multi-layer Encryption | Very High | Jscrambler, custom | Nested decoders |
| 31 | Computed Names | Low | obfuscator.io, js-obfuscator | String concat in names |
| 32 | Bitwise Operators | Low | Custom | `<<`, `>>`, `^`, `\|`, `&` |
| 33 | Ternary Chains | Medium | Custom | Nested `? :` |
| 34 | Function Hoisting | Medium | Custom | Hoisting analysis |
| 35 | Closure State | Medium | Custom | IIFE + captured vars |

---

## Implementation Priorities for js-beautify-rs

### Phase 1 (High Impact, Low Complexity):
- Unicode/Hex Escapes (Techniques 1-2)
- Constant Folding (Technique 7)
- Number Systems (Technique 17)
- Bitwise Operators (Technique 32)

### Phase 2 (Medium Impact, Medium Complexity):
- String Array Rotation (Technique 3)
- String Array Decoder (Technique 4)
- Identifier Renaming (Technique 8)
- Computed Properties (Technique 18)
- Comma Expressions (Technique 19)

### Phase 3 (High Impact, High Complexity):
- Control Flow Flattening (Technique 5)
- eval/Function Execution (Technique 14)
- Packer Wrapper (Technique 22)
- VM Opcodes (Technique 26)

### Phase 4 (Specialized, Very High Complexity):
- JJencode/AAencode (Techniques 23-24)
- WASM Blob (Technique 25)
- Honey Opcodes (Technique 29)
- Multi-layer Encryption (Technique 30)

