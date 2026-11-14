# EXHAUSTIVE AKAMAI BOT MANAGER OBFUSCATION RESEARCH
## Compiled April 17, 2026

---

## PART 1: ANNOTATED REFERENCE LIST (20 Most Useful Sources)

### PRIMARY TECHNICAL ANALYSIS

1. **Analyzing Akamai BMP 4.1.3 - Part 1 & 2 (xve-e, 2026)**
   - URL: https://xve-e.github.io/2026/03/23/analyzing-akamai-bmp-part-2.html
   - Summary: Deep dive into native library (libakamaibmp.so) encryption, LCG substitution cipher, MT19937 PRNG, Speck-like block cipher, JNI entry points, sensor data serialization pipeline
   - **KEY FINDING**: LCG-based string deobfuscation with constant 0x8641, encoded separator "WUfOL#f}+", AES-128-CBC encryption, HMAC validation

2. **Akamai Bot Manager v2 — Technical Analysis (Edioff, 2026)**
   - URL: https://github.com/Edioff/akamai-analysis
   - Summary: 512KB obfuscated JavaScript analysis, 100+ browser signals, string array rotation, control flow flattening, dead code injection patterns
   - **KEY FINDING**: String obfuscation via rotated array with numeric offsets, timing traps for debugger detection, canvas/WebGL fingerprinting

3. **Akamai v3 Sensor Data Deep Dive (glizzykingdreko, 2025)**
   - URL: https://medium.com/@glizzykingdreko/akamai-v3-sensor-data-deep-dive-into-encryption-decryption-and-bypass-tools-da0adad2a784
   - Summary: v3 encryption (element shuffling + character substitution), PRNG-based encryption, file hash extraction via AST, cookie hash integration
   - **KEY FINDING**: v3 uses script-dependent file hash + cookie hash for encryption, unlike v2's static approach

4. **Decoding Akamai 2.0 Anti-Scraping (小伟, 2023)**
   - URL: https://medium.com/@240942649/decoding-akamai-2-0-418e7c7fa0a0
   - Summary: 58-element array encryption, canvas fingerprint + motion trajectory as critical signals, akamai-bm-telemetry header generation
   - **KEY FINDING**: Canvas fingerprint cannot be randomly forged; motion trajectory algorithms essential for realistic bypass

### DEOBFUSCATOR REPOSITORIES

5. **voidstar0/akamai-deobfuscator (312 stars, JavaScript)**
   - URL: https://github.com/char/akamai-deobfuscator
   - Summary: Function/property name recovery, proxy function removal, control flow unflattening, string concealing removal
   - Status: Archived (2023), but foundational for understanding deobfuscation patterns

6. **manudeobs/Akamai-2.0-Deobfuscator (70 stars, JavaScript)**
   - URL: https://github.com/manudeobs/Akamai-2.0-Deobfuscator
   - Summary: Integrity check patching, proxy function removal, control flow unflattening, string concealing removal
   - **KEY FINDING**: Babel-based AST manipulation for deobfuscation

7. **rastvl/akamai-deobfuscator-2.0 (83 stars, JavaScript)**
   - URL: https://github.com/rastvl/akamai-deobfuscator-2.0
   - Summary: Dynamic script deobfuscator from ASOS, string replacement focus, proxy function handling
   - Note: Recommends deobfuscate.io for proxy function replacement

8. **luluhoc/akamai_v2_toolkit (76 stars, JavaScript/TypeScript)**
   - URL: https://github.com/luluhoc/akamai_v2_toolkit
   - Summary: Sensor data decryptor, ASCII special character handling, BMAK object analysis
   - **KEY FINDING**: Uses ASCII character set [32-126] excluding quotes and backslash

### SENSOR DATA & ENCRYPTION

9. **glizzykingdreko/akamai-v3-sensor-data-helper (npm module)**
   - URL: https://github.com/glizzykingdreko/akamai-v3-sensor-data-helper
   - Summary: Babel AST parsing for file hash extraction, PRNG implementation, element swapping + character substitution
   - **KEY FINDING**: Comprehensive npm module for v3 encryption/decryption

10. **glizzykingdreko/akamai-sensordata-decryptor (npm CLI tool)**
    - URL: https://github.com/glizzykingdreko/akamai-sensordata-decryptor
    - Summary: v2 sensor data decryption, character substitution reversal, data shuffling reversal
    - **KEY FINDING**: Detailed encryption/decryption process documentation

11. **xiaoweigege/akamai2.0-sensor_data (113 stars, JavaScript)**
    - URL: https://github.com/xiaoweigege/akamai2.0-sensor_data
    - Summary: sensor_data parameter analysis, akamai-bm-telemetry generation, JA3 fingerprinting bypass
    - **KEY FINDING**: 58-element array structure, canvas fingerprint criticality

### BMP GENERATORS & MOBILE ANALYSIS

12. **xvertile/akamai-bmp-generator (315 stars, Go)**
    - URL: https://github.com/xvertile/akamai-bmp-generator
    - Summary: Fully reversed BMP implementation, supports v2.1.2-3.3.4, 2K device fingerprints, PoW support
    - **KEY FINDING**: Multi-version support, device fingerprint database

13. **yoghurtbot/akamai-bmp-decrypt (22 stars, JavaScript)**
    - URL: https://github.com/yoghurtbot/akamai-bmp-decrypt
    - Summary: Mobile SDK sensor string decryption, RSA + AES-128-CBC encryption format
    - **KEY FINDING**: Format: "1,a,{rsa1},{rsa2}${base64(IV+cipher+HMAC)}${timing}"

14. **yoghurtbot/akamai-bmp-rsa-aes-frida-hook (67 stars, JavaScript)**
    - URL: https://github.com/yoghurtbot/akamai-bmp-rsa-aes-frida-hook
    - Summary: Frida hooks for RSA key extraction, AES encryption inspection, native library analysis
    - **KEY FINDING**: Hooks into Crypto::RSAEncrypt and AES routines in libakamaibmp.so

15. **ignassew/bapka (30 stars, Python)**
    - URL: https://github.com/ignassew/bapka
    - Summary: APK analyzer for BMP version detection, comprehensive app database
    - **KEY FINDING**: Identifies BMP versions across 20+ Android apps

### FINGERPRINTING & BYPASS TECHNIQUES

16. **Fingerprinting Beyond JA3 (Ijaz Ur Rahim, 2025)**
    - URL: https://ijazurrahim.com/blog/fingerprinting-beyond-ja3.html
    - Summary: HTTP/2 fingerprinting (SETTINGS, WINDOW_UPDATE, priorities, pseudo-header order), JA3 brittleness, Akamai's signature technique
    - **KEY FINDING**: Akamai format: "S[settings]|WU[window_update]|P[priorities]|PS[pseudo_header_order]"

17. **TLS Fingerprinting in 2026: JA3, JA4+ (packet.guru, 2026)**
    - URL: https://packet.guru/blog/TLS-Fingerprinting-JA3-JA4
    - Summary: JA3 vs JA4+ comparison, GREASE, ECH encryption, WAF adaptation strategies
    - **KEY FINDING**: JA4 structured format (a_b_c) more resilient than JA3 hashing

18. **How to Bypass Akamai in 2026 (dev.to, 2026)**
    - URL: https://dev.to/vhub_systems_ed5641f65d59/how-to-bypass-akamai-bot-detection-in-2026-39lj
    - Summary: TLS fingerprinting (JA3), HTTP/2 fingerprinting, behavioral analysis, bot score heuristics
    - **KEY FINDING**: curl-cffi with browser impersonation drops detection from 100% to 0% on TLS-only sites

19. **How to Bypass Akamai when Web Scraping (Scrapfly, 2026)**
    - URL: https://scrapfly.io/blog/how-to-bypass-akamai-anti-scraping/
    - Summary: Multi-layer detection (TLS, IP reputation, JavaScript fingerprinting), residential proxy strategies
    - **KEY FINDING**: JavaScript fingerprinting most complex layer; requires real browser execution

20. **Bypass Akamai Bot Manager with Mobile Proxies (DataResearchTools, 2026)**
    - URL: https://dataresearchtools.com/bypass-akamai-proxy/
    - Summary: Device fingerprinting depth, canvas rendering, WebGL queries, behavioral simulation
    - **KEY FINDING**: Canvas fingerprinting varies by GPU/driver/OS; cannot be spoofed without real data

---

## PART 2: DISTINCT DEOBFUSCATION TRANSFORMATIONS

### STRING OBFUSCATION TECHNIQUES

1. **String Array Rotation**
   - Strings stored in array, accessed via rotated indices
   - Runtime decoder uses numeric offset to retrieve correct string
   - Example: `_0x8b75 = ["Hello", "log"]; console[_0x8b75[1]]`

2. **String Array Shuffling**
   - Array elements shuffled at obfuscation time
   - Decoder must know shuffle pattern or reverse it
   - Akamai v3: Uses PRNG seeded with file hash

3. **Hex Encoding (\\x notation)**
   - Characters encoded as hex: `"log"` → `"\x6c\x6f\x67"`
   - Decoder: `String.fromCharCode(0x6c, 0x6f, 0x67)`

4. **Unicode Escape Sequences (\\u notation)**
   - Characters encoded as Unicode: `"log"` → `"\u006c\u006f\u0067"`
   - Decoder: `String.fromCharCode(0x006c, 0x006f, 0x0067)`

5. **Base64 Encoding**
   - Strings base64-encoded: `"log"` → `"bG9n"`
   - Decoder: `atob("bG9n")`

6. **XOR Cipher**
   - String XORed with key: `encrypt(str, key) = str.charCodeAt(i) ^ key.charCodeAt(i % key.length)`
   - Akamai v2: Simple XOR with rotating key offset
   - Example key: `'bj4[sd[wdsJMmJN+xK=t+wR]7"DcID'`

7. **LCG Substitution Cipher (Akamai Native)**
   - Linear Congruential Generator-based character substitution
   - Charset: ASCII [32-126] excluding `"`, `'`, `\`
   - Seed: 0x8641 (constant)
   - Used for: RSA key deobfuscation, separator decoding

8. **Character Substitution with PRNG**
   - Akamai v3: Each character substituted using PRNG-generated offset
   - PRNG seeded with cookie hash
   - Reversible by reinitializing PRNG with same seed

### CONTROL FLOW OBFUSCATION

9. **Control Flow Flattening**
   - Linear code transformed into switch-case dispatcher
   - Each code block becomes case statement
   - Dispatcher state machine controls execution order
   - Opaque predicates determine next state
   - Example: `if (x > 5) { a(); } else { b(); }` → `switch(state) { case 1: if(opaque) state=2; else state=3; break; case 2: a(); state=4; break; case 3: b(); state=4; break; }`

10. **Dead Code Injection**
    - Unreachable code blocks inserted
    - Fake branches that never execute
    - Decoy functions with no callers
    - Increases code size and analysis difficulty

11. **Opaque Predicates**
    - Conditions with known but obfuscated outcomes
    - Example: `if (1 === 1)` always true, but written as `if (Math.random() > -1)`
    - Used to create fake branches in control flow

12. **Irreducible Control Flow**
    - Code structure that cannot be reduced to simple loops/conditionals
    - Multiple entry/exit points in same block
    - Defeats automated decompilers

### IDENTIFIER OBFUSCATION

13. **Variable Name Mangling**
    - Meaningful names replaced with cryptic identifiers
    - Example: `calculateTotal` → `_0x2a4b` or `a`
    - Single-letter names: `a`, `b`, `c`, etc.
    - Hex patterns: `_0x1f9a`, `_0x4c8b`

14. **Function Name Mangling**
    - Same as variable mangling
    - Breaks semantic understanding of code

15. **Property Name Obfuscation**
    - Object properties renamed
    - Example: `user.name` → `user._0x1f9a`

### PROPERTY ACCESS OBFUSCATION

16. **Bracket Notation Conversion**
    - Dot notation converted to bracket notation
    - Example: `object.property` → `object["property"]`
    - Further obfuscated: `object[_0x1f9a('0x3b')]`
    - Defeats simple string matching

17. **Dynamic Property Access**
    - Property names computed at runtime
    - Example: `obj[getPropertyName()]` instead of `obj.property`

### NUMERIC OBFUSCATION

18. **Hexadecimal Number Literals**
    - Decimal numbers converted to hex
    - Example: `255` → `0xFF`, `1000` → `0x3E8`
    - Decoder: Automatic by JavaScript engine

19. **Number to Expression Conversion**
    - Numbers replaced with arithmetic expressions
    - Example: `10` → `5 + 5` or `20 / 2`
    - Increases code size and complexity

### ENCRYPTION-BASED OBFUSCATION

20. **AES-128-CBC Encryption (Akamai Mobile)**
    - Sensor data encrypted with AES-128-CBC
    - IV + ciphertext + HMAC format
    - RSA-encrypted AES key

21. **HMAC Validation**
    - Sensor data integrity verified with HMAC
    - Prevents tampering detection

22. **Polymorphic Dispatcher (Akamai Native)**
    - Single function handles multiple operations
    - Determined by parameters and constants
    - Example: `sub_25E0AC(string_ptr, 0x8641, 0xFFFFFFFF)` → string deobfuscation

### ANTI-ANALYSIS TECHNIQUES

23. **Debugger Detection**
    - Timing checks: Execution time anomalies indicate debugging
    - DevTools detection: Checks for open developer tools
    - Breakpoint detection: Monitors for breakpoint insertion

24. **Prototype Poisoning Detection**
    - Checks if native JavaScript prototypes modified
    - Detects monkey-patching attempts

25. **Timing Traps**
    - Code measures execution time of operations
    - Abnormal timing indicates non-standard environment
    - Used to detect headless browsers, emulators

---

## PART 3: CANONICAL AST PATTERNS FROM WRITEUPS

### String Array Rotation Pattern
```javascript
// Obfuscated pattern
var _0x8b75 = ["Hello", "world", "log"];
function _0x2d8f(a) {
  return _0x8b75[a % _0x8b75.length];
}
console[_0x2d8f('0x2')](_0x2d8f('0x0') + ' ' + _0x2d8f('0x1'));

// Deobfuscated
console.log("Hello world");
```

### Control Flow Flattening Pattern
```javascript
// Obfuscated pattern
var _0x4c8b = function() {
  var _0x1f9a = 0;
  while (true) {
    switch (_0x1f9a) {
      case 0:
        console.log("Step 1");
        _0x1f9a = 1;
        break;
      case 1:
        console.log("Step 2");
        _0x1f9a = 2;
        break;
      case 2:
        return;
    }
  }
};

// Deobfuscated
function _0x4c8b() {
  console.log("Step 1");
  console.log("Step 2");
}
```

### Bracket Notation Pattern
```javascript
// Obfuscated
var _0x1f9a = { 'property': 'value' };
var _0x4c8b = _0x1f9a['\x70\x72\x6f\x70\x65\x72\x74\x79'];

// Deobfuscated
var _0x1f9a = { property: 'value' };
var _0x4c8b = _0x1f9a.property;
```

### Dead Code Injection Pattern
```javascript
// Obfuscated
function _0x2a4b() {
  var _0x1c5d = 0;
  if (_0x1c5d > 100) {
    console.log("Never executes");
    return;
  }
  console.log("Real code");
}

// Deobfuscated
function _0x2a4b() {
  console.log("Real code");
}
```

### LCG Substitution Cipher (Akamai Native)
```python
# Deobfuscation pattern
def build_charset():
    return [ch for ch in range(32, 127) if ch not in (34, 39, 92)]

def lcg_decode(data: bytes, seed: int) -> bytes:
    charset = build_charset()
    n = len(charset)
    lcg = seed
    out = []
    for byte in data:
        idx = charset.index(byte)
        shift = ((lcg >> 8) & 0xFFFF) % n
        out.append(charset[(idx - shift) % n])
        lcg = (lcg * 1103515245 + 12345) & 0xFFFFFFFF
    return bytes(out)

# Usage: lcg_decode(b"WUfOL#f}+", 0x8641) → separator string
```

### Akamai v3 Element Shuffling Pattern
```javascript
// Encryption pattern (simplified)
function encryptSensorData(payload, fileHash, cookieHash) {
  // Step 1: Element shuffling using file hash PRNG
  let shuffled = shuffleElements(payload, fileHash);
  
  // Step 2: Character substitution using cookie hash PRNG
  let encrypted = substituteCharacters(shuffled, cookieHash);
  
  return encrypted;
}

// Decryption pattern
function decryptSensorData(encrypted, fileHash, cookieHash) {
  // Step 1: Reverse character substitution
  let unsubstituted = reverseSubstitution(encrypted, cookieHash);
  
  // Step 2: Reverse element shuffling
  let payload = unshuffleElements(unsubstituted, fileHash);
  
  return payload;
}
```

### Akamai Sensor Data Structure
```javascript
// 58-element array structure (v2)
var sensorData = [
  // Browser fingerprint (20+ elements)
  userAgent,
  screenResolution,
  colorDepth,
  timezone,
  language,
  plugins,
  // Hardware (5+ elements)
  cpuCount,
  memory,
  // Behavior (15+ elements)
  mouseMovements,
  clickPatterns,
  scrollBehavior,
  // Canvas fingerprint
  canvasHash,
  // WebGL fingerprint
  webglVendor,
  // Timing
  navigationTiming,
  // ... more elements
];

// Encrypted format
var encrypted = encryptAES(sensorData, aesKey, iv);
var hmac = computeHMAC(encrypted, hmacKey);
var payload = base64(iv + encrypted + hmac);
```

---

## PART 4: AKAMAI-SPECIFIC VARIABLE/FUNCTION NAMES (SIGNATURES)

### Cookie Names
- `_abck` - Primary validation cookie (set after sensor data submission)
- `bm_sz` - Cookie hash container (used in v3 encryption)
- `ak_bmsc` - Akamai Bot Manager Session Cookie
- `bm_sv` - Bot Manager Session Validation

### JavaScript Object Names
- `bmak` - Main Bot Manager object
- `bmak.xa` - Sensor data generation function
- `bmak.sensor_data` - Sensor payload property
- `bmak.ke` - Key exchange function
- `cf_` - Cloudflare-style prefix (sometimes used)
- `cv_` - Canvas validation prefix

### Function Name Patterns
- `_abck_*` - Cookie-related functions
- `sensor_*` - Sensor collection functions
- `encrypt_*` - Encryption functions
- `fingerprint_*` - Fingerprinting functions
- `validate_*` - Validation functions

### Header Names
- `x-acf-sensor-data` - Mobile SDK sensor data header
- `akamai-bm-telemetry` - Telemetry header (base64-encoded sensor data)
- `x-akamai-*` - Generic Akamai headers

### Endpoint Patterns
- `/_sec/cp_challenge/` - Challenge endpoint
- `/_sec/cp_challenge/verify` - Sensor data submission endpoint
- `/api/sensor` - Alternative sensor endpoint

### Payload Format Signatures
- `sensor_data=` - Query parameter for sensor data
- `2;` or `3;` - Version prefix in sensor_data
- `{rsa1},{rsa2}$` - RSA key format in mobile BMP
- `bm_sz=` - Cookie hash parameter

---

## PART 5: OBFUSCATION TOOLING ANALYSIS

### Likely Obfuscator: Jscrambler or Custom Derivative

**Evidence:**
1. **Control Flow Flattening**: Akamai uses sophisticated switch-case flattening (Jscrambler signature)
2. **Polymorphic Behavior**: Each deployment generates different obfuscated output (Jscrambler feature)
3. **String Array Rotation**: Rotated string arrays with numeric offsets (Jscrambler standard)
4. **Dead Code Injection**: Unreachable branches and decoy functions (Jscrambler feature)
5. **Bracket Notation Conversion**: Dot notation → bracket notation (Jscrambler transformation)
6. **Self-Defending Code**: Runtime integrity checks, debugger detection (Jscrambler RASP)

### Jscrambler Transformations Observed in Akamai

| Transformation | Evidence | Akamai Usage |
|---|---|---|
| String Array | Rotated array with offsets | Primary string obfuscation |
| Control Flow Flattening | Switch-case dispatcher | Main control flow technique |
| Dead Code Injection | Unreachable branches | Complexity increase |
| Identifier Renaming | _0x2a4b, _0x1c5d patterns | Variable/function names |
| Dot to Bracket | object.prop → object["prop"] | Property access |
| Self Defending | Debugger detection, timing traps | Anti-analysis |
| Opaque Predicates | Fake conditions | Control flow obfuscation |

### Custom Additions (Akamai-Specific)

1. **LCG Substitution Cipher**: Custom implementation for native code
2. **Polymorphic Dispatcher**: Single function handles multiple operations
3. **Speck-like Block Cipher**: Custom encryption in native library
4. **MT19937 PRNG**: Mersenne Twister for sensor data generation
5. **Dynamic Script Generation**: Script changes between deployments

---

## PART 6: BMP IDENTIFICATION MARKERS/SIGNATURES

### JavaScript Markers
```javascript
// Presence of these indicates Akamai BMP
- window._abck
- window.bm_sz
- window.ak_bmsc
- window.bmak
- /_sec/cp_challenge/ in network requests
- x-acf-sensor-data header (mobile)
- akamai-bm-telemetry header
```

### Network Markers
```
HTTP Status: 403 with "Pardon Our Interruption" message
Cookies: _abck, bm_sz, ak_bmsc
Headers: x-akamai-*, akamai-bm-telemetry
Endpoints: /_sec/cp_challenge/verify
```

### Code Patterns
```javascript
// Akamai-specific patterns
- String array with 500+ entries
- Switch statement with 100+ cases
- Hex-encoded strings (\x notation)
- LCG-based deobfuscation (0x8641 constant)
- Canvas fingerprinting code
- WebGL vendor/renderer queries
- Motion sensor access attempts
```

### Mobile Markers
```
Header: x-acf-sensor-data
Format: 1,a,{rsa1},{rsa2}${base64}${timing}
Library: libakamaibmp.so (Android)
Package: com.cyberfend.cyfsecurity (Java side)
JNI Functions: buildN, encryptKeyN, decryptN
```

---

## PART 7: VERSION-SPECIFIC PATTERNS

### Akamai BMP v2.x
- Simple XOR cipher for strings
- Basic control flow flattening
- 58-element sensor array
- _abck cookie validation
- No native library (JavaScript only)

### Akamai BMP v3.x
- LCG substitution cipher (native)
- Advanced control flow flattening
- Polymorphic dispatcher
- RSA + AES-128-CBC encryption
- Native library (libakamaibmp.so)
- Proof-of-Work support

### Akamai BMP v4.x
- Speck-like block cipher (custom)
- MT19937 PRNG
- Encrypted native library on disk
- 12+ sensor subsystems
- Dynamic script generation
- Enhanced anti-analysis

---

## PART 8: RECOMMENDED DEOBFUSCATION STRATEGY FOR RUST

### Phase 1: String Deobfuscation
1. Detect string array pattern (array of 500+ strings)
2. Identify rotation/shuffle pattern
3. Implement array index mapping
4. Decode hex/unicode escapes
5. Reverse XOR/LCG ciphers

### Phase 2: Control Flow Analysis
1. Identify switch-case dispatcher
2. Map state transitions
3. Reconstruct linear code flow
4. Remove dead code branches
5. Simplify opaque predicates

### Phase 3: Identifier Recovery
1. Track variable usage patterns
2. Infer semantic meaning from context
3. Rename to meaningful identifiers
4. Preserve function signatures

### Phase 4: Native Code Analysis
1. Detect Speck-like cipher
2. Implement LCG substitution
3. Extract RSA keys (via Frida hooks)
4. Decrypt AES payloads
5. Validate HMAC signatures

---

## CONCLUSION

Akamai Bot Manager uses a sophisticated multi-layer obfuscation strategy combining:
- **Jscrambler-like transformations** (control flow flattening, string arrays, dead code)
- **Custom cryptographic implementations** (LCG, Speck-like cipher, MT19937)
- **Anti-analysis techniques** (debugger detection, timing traps, prototype poisoning checks)
- **Dynamic generation** (polymorphic output, version-specific changes)

The most critical signals for deobfuscation are:
1. String array rotation with numeric offsets
2. Switch-case control flow dispatcher
3. LCG constant 0x8641 (native code)
4. Sensor data structure (58 elements, canvas fingerprint)
5. Cookie names (_abck, bm_sz, ak_bmsc)

