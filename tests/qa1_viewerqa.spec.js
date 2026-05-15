const { test, expect } = require("@playwright/test");

const BASE_URL = "http://localhost:8080";

test("qa-1 viewer qa surface contract", async ({ page }) => {
  await page.goto(`${BASE_URL}/?qa=1`, { waitUntil: "domcontentloaded" });

  const qaType = await page.evaluate(() => typeof window.__viewerQa);
  expect(qaType).toBe("object");

  const versionType = await page.evaluate(() => typeof window.__viewerQa.version());
  expect(versionType).toBe("number");

  const state = await page.evaluate(() => window.__viewerQa.state());
  expect(typeof state).toBe("object");
  expect(Array.isArray(state)).toBeFalsy();

  const logs = await page.evaluate(() => window.__viewerQa.logs());
  expect(typeof logs).toBe("object");
  expect(Array.isArray(logs.events)).toBeTruthy();

  const metrics = await page.evaluate(() => window.__viewerQa.metrics());
  expect(typeof metrics).toBe("object");
  expect(Array.isArray(metrics)).toBeFalsy();

  const lastError = await page.evaluate(() => window.__viewerQa.lastError());
  expect(lastError === null || typeof lastError === "object").toBeTruthy();

  const waitReadyResolved = await page.evaluate(async () => {
    try {
      await window.__viewerQa.waitForReady({ timeoutMs: 100 });
      return true;
    } catch {
      return false;
    }
  });
  expect(waitReadyResolved).toBeTruthy();

  await page.goto(`${BASE_URL}/`, { waitUntil: "domcontentloaded" });
  const noQaType = await page.evaluate(() => typeof window.__viewerQa);
  expect(noQaType).toBe("undefined");
});
