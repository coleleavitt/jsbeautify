"use strict";
const test = require("node:test");
const assert = require("node:assert");

const patterns = require("../lib/patterns.js");

test("registrableDomain: plain two-label domains", () => {
  assert.strictEqual(patterns.registrableDomain("example.com"), "example.com");
  assert.strictEqual(patterns.registrableDomain("EXAMPLE.COM"), "example.com");
  assert.strictEqual(patterns.registrableDomain("example.com."), "example.com");
});

test("registrableDomain: subdomains collapse to eTLD+1", () => {
  assert.strictEqual(patterns.registrableDomain("cdn.assets.example.com"), "example.com");
  assert.strictEqual(patterns.registrableDomain("a.b.c.example.org"), "example.org");
});

test("registrableDomain: multi-part public suffixes", () => {
  assert.strictEqual(patterns.registrableDomain("shop.example.co.uk"), "example.co.uk");
  assert.strictEqual(patterns.registrableDomain("foo.bar.example.com.au"), "example.com.au");
  assert.strictEqual(patterns.registrableDomain("app.example.vercel.app"), "example.vercel.app");
  assert.strictEqual(patterns.registrableDomain("user.github.io"), "user.github.io");
});

test("registrableDomain: IPs and localhost pass through", () => {
  assert.strictEqual(patterns.registrableDomain("127.0.0.1"), "127.0.0.1");
  assert.strictEqual(patterns.registrableDomain("localhost"), "localhost");
});

test("registrableDomain: empty / invalid", () => {
  assert.strictEqual(patterns.registrableDomain(""), "");
  assert.strictEqual(patterns.registrableDomain(null), "");
});

test("hostnameForURL extracts and normalizes host", () => {
  assert.strictEqual(patterns.hostnameForURL("https://CDN.Example.com/app.js?x=1"), "cdn.example.com");
  assert.strictEqual(patterns.hostnameForURL("not a url"), "");
});
