const { test, expect } = require("@playwright/test");

const BASE_URL = "http://localhost:8080";

// The viewer renders with WebGPU. Without an adapter the app never initializes, and a spec that
// accepts that outcome passes vacuously. Launch with WebGPU (macOS defaults below; override with
// QA_CHROME_ARGS) and fail when the app is not ready, unless QA_ALLOW_NO_GPU=1 is set explicitly.
const ALLOW_NO_GPU = process.env.QA_ALLOW_NO_GPU === "1";
test.use({
  launchOptions: {
    args: (process.env.QA_CHROME_ARGS ?? "--enable-unsafe-webgpu --use-angle=metal")
      .split(" ")
      .filter(Boolean),
  },
});

function assertAppReady(state, lastError) {
  if (state?.qa?.ready || ALLOW_NO_GPU) return;
  throw new Error(
    `App did not become ready (${lastError?.category ?? "no error"}: ${lastError?.message ?? ""}). ` +
      "This spec needs a WebGPU-capable Chrome; set QA_ALLOW_NO_GPU=1 to accept adapter failures.",
  );
}

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

  // Wait until the app is ready or has reported why it cannot be, rather than a fixed delay.
  await page.waitForFunction(
    () => {
      const qa = window.__viewerQa;
      return qa?.state?.()?.qa?.ready === true || !!qa?.lastError?.();
    },
    {},
    { timeout: 10000 },
  );
  const readyState = await page.evaluate(() => window.__viewerQa.state());
  const readyError = await page.evaluate(() => window.__viewerQa.lastError());
  assertAppReady(readyState, readyError);
  if (!readyState.qa.ready) {
    expect(readyError).toBeTruthy();
    expect(["wgpu.adapter", "wgpu.device", "wgpu.surface"]).toContain(readyError.category);
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

  assertAppReady(state, lastError);
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
    expect(
      activeRoi.last_completed_job === null ||
        typeof activeRoi.last_completed_job === "string",
    ).toBeTruthy();
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

  assertAppReady(state, lastError);
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

test("qa-4 liver geometry, orientation letters, and viewport facts", async ({ page }) => {
  await page.goto(`${BASE_URL}/?qa=1&sample=liver_0&preset=image_label_mpr_basic`, {
    waitUntil: "domcontentloaded",
  });
  await page.waitForFunction(() => typeof window.__viewerQa === "object");
  await page.waitForFunction(
    () => {
      const qa = window.__viewerQa;
      return qa?.state?.()?.qa?.ready === true || !!qa?.lastError?.();
    },
    {},
    { timeout: 30000 },
  );
  const { state, lastError, metrics } = await page.evaluate(() => ({
    state: window.__viewerQa.state(),
    lastError: window.__viewerQa.lastError(),
    metrics: window.__viewerQa.metrics(),
  }));
  assertAppReady(state, lastError);
  if (!state.qa.ready) return; // QA_ALLOW_NO_GPU: nothing more to assert without a renderer.

  // Geometry comes from the sample's sform: 2 x 2 x 3 mm voxels, RAS storage.
  expect(state.volume.dimensions).toEqual([180, 180, 125]);
  expect(state.volume.spacing).toEqual([2, 2, 3]);
  expect(state.volume.orientation).toEqual([0, 0, 0, 1]);

  // The sample holds liver (1) and tumor (2); each becomes its own ROI instead of merging.
  expect(state.rois.map((roi) => roi.name)).toEqual([
    "liver_0_label.nii [label 1]",
    "liver_0_label.nii [label 2]",
  ]);
  expect(state.rois.filter((roi) => roi.active).map((roi) => roi.name)).toEqual([
    "liver_0_label.nii [label 1]",
  ]);
  expect(state.rois.every((roi) => roi.authority === "Voxel" && roi.visible)).toBe(true);
  expect(metrics.visible_rois).toBe(2);

  // Edge letters are derived from the affine (left, right, top, bottom).
  const byMode = Object.fromEntries(state.viewports.map((vp) => [vp.mode, vp]));
  expect(byMode.axial.edge_letters).toBe("RLAP");
  expect(byMode.coronal.edge_letters).toBe("RLSI");
  expect(byMode.sagittal.edge_letters).toBe("APSI");

  // The MPR preset puts the cursor inside the liver, and every 2D view can draw the overlay.
  for (const mode of ["axial", "coronal", "sagittal"]) {
    expect(byMode[mode].cursor_intersects_active_roi, mode).toBe(true);
    expect(byMode[mode].volume_slice_in_bounds, mode).toBe(true);
    expect(byMode[mode].overlay_renderable, mode).toBe(true);
  }
  expect(byMode.oblique.overlay_renderable).toBe(true);
  expect(byMode.three_d.image_renderable).toBe(true);

  // Contours and the 3D mesh must actually be drawn, not just be renderable: the batch counts
  // are what the renderer submitted (regression: a stale 1x1 surface size clipped both to nothing).
  await page.waitForFunction(
    () => {
      const render = window.__viewerQa.state().render;
      return render.contour_batch_count > 0 && render.mesh_batch_count > 0;
    },
    null,
    { timeout: 15000 },
  );

  // A healthy load reports no errors or warnings.
  expect(lastError).toBeNull();
  expect(metrics.error_count).toBe(0);
  expect(metrics.warning_count).toBe(0);
});

test("qa-5 the 3D view is cached and only re-marched when its image changes", async ({ page }) => {
  await page.goto(`${BASE_URL}/?qa=1&sample=liver_0&preset=image_label_mpr_basic`, {
    waitUntil: "domcontentloaded",
  });
  await page.waitForFunction(() => typeof window.__viewerQa === "object");
  await page.waitForFunction(() => window.__viewerQa.state()?.qa?.ready === true, null, {
    timeout: 30000,
  });
  // Let the load, mesh build, and the full-quality settle frame finish.
  await page.waitForTimeout(2500);

  const marches = () => page.evaluate(() => window.__viewerQa.state().render.view3d_march_count);
  // Late load work (a mesh or contour finishing) may still re-march once; wait until the count
  // has held still for 1.5 s so the check below only sees what the interaction causes.
  let before = await marches();
  for (let stableSince = Date.now(); Date.now() - stableSince < 1500; ) {
    await page.waitForTimeout(250);
    const now = await marches();
    if (now !== before) {
      before = now;
      stableSince = Date.now();
    }
  }
  expect(before).toBeGreaterThan(0);

  // Scrolling a 2D view moves the cursor, which only moves the crosshair drawn over the cache.
  const axial = await page.evaluate(() =>
    window.__viewerQa.state().viewports.find((vp) => vp.mode === "axial").rect,
  );
  const scale = await page.evaluate(() => window.devicePixelRatio);
  await page.mouse.move((axial[0] + axial[2] / 2) / scale, (axial[1] + axial[3] / 2) / scale);
  for (let i = 0; i < 20; i++) {
    await page.mouse.wheel(0, i % 2 ? 120 : -120);
    await page.waitForTimeout(20);
  }
  await page.waitForTimeout(500);
  expect(await marches(), "2D interaction must not re-march the 3D view").toBe(before);

  // Zooming the 3D view changes its image and must re-march it.
  const threeD = await page.evaluate(() =>
    window.__viewerQa.state().viewports.find((vp) => vp.mode === "three_d").rect,
  );
  await page.mouse.move((threeD[0] + threeD[2] / 2) / scale, (threeD[1] + threeD[3] / 2) / scale);
  await page.keyboard.down("Control");
  for (let i = 0; i < 4; i++) {
    await page.mouse.wheel(0, -120);
    await page.waitForTimeout(30);
  }
  await page.keyboard.up("Control");
  await page.waitForTimeout(800);
  expect(await marches()).toBeGreaterThan(before);
});

test("qa-6 every visible ROI shows its derived mesh and contours", async ({ page }) => {
  await page.goto(`${BASE_URL}/?qa=1&sample=liver_0&preset=image_label_mpr_basic`, {
    waitUntil: "domcontentloaded",
  });
  await page.waitForFunction(() => typeof window.__viewerQa === "object");
  await page.waitForFunction(() => window.__viewerQa.state()?.qa?.ready === true, null, {
    timeout: 30000,
  });
  // Every visible ROI is built, one after another, so wait for all of them.
  await page.waitForFunction(
    () => {
      const rois = window.__viewerQa.state().rois.filter((roi) => roi.visible);
      return rois.length >= 2 && rois.every((roi) => roi.mesh_cache_current);
    },
    null,
    { timeout: 30000 },
  );
  const { rois, render } = await page.evaluate(() => ({
    rois: window.__viewerQa.state().rois,
    render: window.__viewerQa.state().render,
  }));
  const visible = rois.filter((roi) => roi.visible);
  expect(visible.length).toBeGreaterThanOrEqual(2);
  for (const roi of visible) {
    expect(roi.mesh_cache_current, `mesh of ${roi.name}`).toBeTruthy();
  }
  expect(render.mesh_batch_count).toBeGreaterThan(0);
  expect(render.contour_batch_count).toBeGreaterThan(0);
});
