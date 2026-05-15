const { test, expect } = require("@playwright/test");

const BASE_URL = "http://localhost:8080";

test("qa-1 viewer qa surface contract", async ({ page }) => {
  await page.goto(`${BASE_URL}/?qa=1`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => typeof window.__viewerQa === "object");

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

test("qa-2 sample preset readiness", async ({ page }) => {
  await page.goto(
    `${BASE_URL}/?qa=1&sample=liver_0&preset=image_label_mpr_basic`,
    { waitUntil: "domcontentloaded" },
  );
  await page.waitForFunction(() => typeof window.__viewerQa === "object");

  await page.waitForFunction(
    () => {
      const qa = window.__viewerQa;
      if (!qa) return false;
      const state = qa.state?.();
      const err = qa.lastError?.();
      return state?.qa?.ready === true || !!err;
    },
    {},
    { timeout: 10000 },
  );
  const { state, lastError, logs } = await page.evaluate(() => ({
    state: window.__viewerQa.state(),
    lastError: window.__viewerQa.lastError(),
    logs: window.__viewerQa.logs(),
  }));

  expect(state.qa.requested_sample).toBe("liver_0");
  expect(state.qa.requested_preset).toBe("image_label_mpr_basic");

  if (state.qa.ready) {
    const byMode = Object.fromEntries(state.viewports.map((vp) => [vp.mode, vp]));
    for (const mode of ["axial", "coronal", "sagittal", "three_d"]) {
      expect(byMode[mode]).toBeTruthy();
      expect(byMode[mode].rect[2]).toBeGreaterThan(0);
      expect(byMode[mode].rect[3]).toBeGreaterThan(0);
    }
    expect(byMode.axial.overlay_renderable).toBeTruthy();
    expect(byMode.coronal.overlay_renderable).toBeTruthy();
    expect(byMode.sagittal.overlay_renderable).toBeTruthy();

    const activeRoi = state.rois.find((roi) => roi.active);
    expect(activeRoi).toBeTruthy();
    expect(activeRoi.visible).toBeTruthy();
    expect(activeRoi.overlay_slot === 0 || activeRoi.overlay_slot === 1).toBeTruthy();
    expect(lastError).toBeNull();
    expect(logs.events.some((e) => e.category === "qa.sample")).toBeTruthy();
    expect(logs.events.some((e) => e.category === "qa.preset")).toBeTruthy();
    expect(logs.events.some((e) => e.level === "error")).toBeFalsy();
  } else {
    expect(lastError).toBeTruthy();
    expect(["wgpu.adapter", "wgpu.device", "wgpu.surface"]).toContain(lastError.category);
    expect(state.qa.ready).toBeFalsy();
    expect(state.qa.readiness_blockers).toContain("app_context_not_ready");
    expect(logs.events.some((e) => e.level === "error")).toBeTruthy();
  }
});
