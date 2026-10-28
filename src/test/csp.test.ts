// The Tauri CSP forbids inline styles and scripts in HTML: WebView2 silently
// blocks a `style=` attribute or an inline <script>, while a plain browser
// shows nothing wrong. Keep index.html free of both.
import { describe, expect, it } from "vitest";
import html from "../../index.html?raw";

describe("index.html under the Tauri CSP", () => {
  it("has no style attributes", () => {
    expect(html).not.toMatch(/\sstyle\s*=/i);
  });

  it("has no inline scripts and no <style> blocks", () => {
    expect(html).not.toMatch(/<script(?![^>]*\ssrc=)[^>]*>/i);
    expect(html).not.toMatch(/<style[\s>]/i);
  });
});
