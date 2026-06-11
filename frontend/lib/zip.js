(function (global) {
  "use strict";

  const encoder = new TextEncoder();
  const crcTable = new Uint32Array(256);

  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    crcTable[n] = c >>> 0;
  }

  function crc32(bytes) {
    let crc = 0xffffffff;
    for (let i = 0; i < bytes.length; i++) {
      crc = crcTable[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
    }
    return (crc ^ 0xffffffff) >>> 0;
  }

  function u16(value) {
    const out = new Uint8Array(2);
    out[0] = value & 0xff;
    out[1] = (value >>> 8) & 0xff;
    return out;
  }

  function u32(value) {
    const out = new Uint8Array(4);
    out[0] = value & 0xff;
    out[1] = (value >>> 8) & 0xff;
    out[2] = (value >>> 16) & 0xff;
    out[3] = (value >>> 24) & 0xff;
    return out;
  }

  function concat(parts) {
    const total = parts.reduce((sum, part) => sum + part.length, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      out.set(part, offset);
      offset += part.length;
    }
    return out;
  }

  function dosDateTime(date) {
    const year = Math.max(1980, date.getFullYear());
    const dosTime =
      (date.getHours() << 11) |
      (date.getMinutes() << 5) |
      Math.floor(date.getSeconds() / 2);
    const dosDate =
      ((year - 1980) << 9) |
      ((date.getMonth() + 1) << 5) |
      date.getDate();
    return { dosTime, dosDate };
  }

  function bytesForContent(content) {
    if (content instanceof Uint8Array) {
      return content;
    }
    if (content instanceof ArrayBuffer) {
      return new Uint8Array(content);
    }
    return encoder.encode(String(content));
  }

  function createZip(files) {
    const chunks = [];
    const central = [];
    let offset = 0;
    const now = dosDateTime(new Date());

    for (const file of files) {
      const nameBytes = encoder.encode(file.path);
      const data = bytesForContent(file.content);
      const crc = crc32(data);
      const flags = 0x0800;
      const method = 0;

      const localHeader = concat([
        u32(0x04034b50),
        u16(20),
        u16(flags),
        u16(method),
        u16(now.dosTime),
        u16(now.dosDate),
        u32(crc),
        u32(data.length),
        u32(data.length),
        u16(nameBytes.length),
        u16(0),
        nameBytes,
      ]);

      chunks.push(localHeader, data);

      central.push(
        concat([
          u32(0x02014b50),
          u16(20),
          u16(20),
          u16(flags),
          u16(method),
          u16(now.dosTime),
          u16(now.dosDate),
          u32(crc),
          u32(data.length),
          u32(data.length),
          u16(nameBytes.length),
          u16(0),
          u16(0),
          u16(0),
          u16(0),
          u32(0),
          u32(offset),
          nameBytes,
        ])
      );

      offset += localHeader.length + data.length;
    }

    const centralOffset = offset;
    let centralSize = 0;
    for (const entry of central) {
      chunks.push(entry);
      centralSize += entry.length;
    }

    chunks.push(
      concat([
        u32(0x06054b50),
        u16(0),
        u16(0),
        u16(files.length),
        u16(files.length),
        u32(centralSize),
        u32(centralOffset),
        u16(0),
      ])
    );

    return new Blob(chunks, { type: "application/zip" });
  }

  global.MindemonZip = { createZip };
})(globalThis);
