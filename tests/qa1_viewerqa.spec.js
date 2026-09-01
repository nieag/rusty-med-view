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
  if (!waitReadyResolved) {
    const failError = await page.evaluate(() => window.__viewerQa.lastError());
    expect(failError).toBeTruthy();
    expect(["wgpu.adapter", "wgpu.device", "wgpu.surface"]).toContain(
      failError.category,
    );
  }

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
    expect(typeof byMode.axial.image_renderable).toBe("boolean");
    expect(typeof byMode.axial.contour_renderable).toBe("boolean");
    expect(typeof byMode.axial.mesh_renderable).toBe("boolean");

    const activeRoi = state.rois.find((roi) => roi.active);
    expect(activeRoi).toBeTruthy();
    expect(activeRoi.visible).toBeTruthy();
    expect(activeRoi.overlay_slot === 0 || activeRoi.overlay_slot === 1).toBeTruthy();
    for (const field of [
      "last_queue_delay_ms",
      "last_contour_raster_ms",
      "last_mesh_voxelization_ms",
      "last_cpu_cache_install_ms",
      "last_gpu_upload_ms",
      "last_work_convergence_ms",
    ]) {
      expect(typeof activeRoi[field]).toBe("number");
    }
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

test("qa-3 structured render facts", async ({ page }) => {
  await page.goto(
    `${BASE_URL}/?qa=1&sample=liver_0&preset=image_label_mpr_basic`,
    { waitUntil: "domcontentloaded" },
  );
  await page.waitForFunction(() => typeof window.__viewerQa === "object");
  let waitTimedOut = false;
  try {
    await page.waitForFunction(
      () => {
        const qa = window.__viewerQa;
        if (!qa) return false;
        const state = qa.state?.();
        const err = qa.lastError?.();
        const modes = new Set((state?.viewports ?? []).map((v) => v.mode));
        const hasAllModes = ["axial", "coronal", "sagittal", "three_d"].every((m) =>
          modes.has(m),
        );
        return (state?.qa?.ready === true && hasAllModes) || !!err;
      },
      {},
      { timeout: 10000 },
    );
  } catch {
    waitTimedOut = true;
  }

  const { state, logs, lastError } = await page.evaluate(() => ({
    state: window.__viewerQa.state(),
    logs: window.__viewerQa.logs(),
    lastError: window.__viewerQa.lastError(),
  }));
  if (waitTimedOut) {
    throw new Error(
      JSON.stringify(
        {
          reason: "qa3_wait_timeout",
          readiness_blockers: state?.qa?.readiness_blockers ?? [],
          viewport_modes: (state?.viewports ?? []).map((v) => v.mode),
          logs: logs?.events ?? [],
        },
        null,
        2,
      ),
    );
  }

  expect(typeof state.render.frame_counter).toBe("number");
  expect(state.render.overlay_slots_max).toBeGreaterThanOrEqual(1);
  expect(typeof state.render.viewport_uniform_count).toBe("number");
  expect(typeof state.render.contour_batch_count).toBe("number");
  expect(typeof state.render.mesh_batch_count).toBe("number");
  expect(state.render.last_warning === null || typeof state.render.last_warning === "string").toBeTruthy();
  expect(state.render.last_error === null || typeof state.render.last_error === "string").toBeTruthy();
  expect(state.render.overlay_slots_used).toBeLessThanOrEqual(state.render.overlay_slots_max);

  if (state.qa.ready) {
    const byMode = Object.fromEntries(state.viewports.map((vp) => [vp.mode, vp]));
    for (const mode of ["axial", "coronal", "sagittal", "three_d"]) {
      expect(byMode[mode]).toBeTruthy();
      expect(typeof byMode[mode].image_renderable).toBe("boolean");
      expect(typeof byMode[mode].overlay_renderable).toBe("boolean");
      expect(typeof byMode[mode].contour_renderable).toBe("boolean");
      expect(typeof byMode[mode].mesh_renderable).toBe("boolean");
      expect(Array.isArray(byMode[mode].render_blockers)).toBeTruthy();
    }
    expect(byMode.axial.image_renderable).toBeTruthy();
    expect(byMode.coronal.image_renderable).toBeTruthy();
    expect(byMode.sagittal.image_renderable).toBeTruthy();
    expect(byMode.axial.overlay_renderable).toBeTruthy();
    expect(byMode.coronal.overlay_renderable).toBeTruthy();
    expect(byMode.sagittal.overlay_renderable).toBeTruthy();
    expect(state.render.overlay_slots_used).toBeGreaterThanOrEqual(1);
    expect(logs.events.some((e) => e.level === "error")).toBeFalsy();
    for (const vp of state.viewports) {
      if (!vp.image_renderable || !vp.overlay_renderable || !vp.contour_renderable || !vp.mesh_renderable) {
        expect(vp.render_blockers.length).toBeGreaterThan(0);
      }
    }
  } else {
    expect(lastError).toBeTruthy();
    expect(["wgpu.adapter", "wgpu.device", "wgpu.surface"]).toContain(lastError.category);
    expect(state.qa.ready).toBeFalsy();
    expect(state.qa.readiness_blockers.length).toBeGreaterThan(0);
  }
});
