// @ts-check
const { test, expect } = require("@playwright/test");

test("app mounts and shows branding", async ({ page }) => {
  await page.goto("/");

  // The main mount point should be visible.
  const main = page.locator("#main");
  await expect(main).toBeVisible();

  // The page title should contain the app name (Dioxus web may duplicate it).
  await expect(page).toHaveTitle(/^RustyBench/);

  // Status bar should show "Idle" (a default tab exists on startup).
  await expect(main).toContainText("Idle");
});

test("shows No Device placeholder when no device is connected", async ({ page }) => {
  await page.goto("/");

  // The placeholder should tell the user no device is connected.
  await expect(page.locator("#main")).toContainText("No Device");

  // A "Scan for Devices" button should be visible.
  await expect(page.getByRole("button", { name: /scan for devices/i })).toBeVisible();
});

test("status bar shows application version", async ({ page }) => {
  await page.goto("/");

  // The status bar should include the version string.
  await expect(page.locator("#main")).toContainText("RustyBench v0.3.0");
});

test("theme toggle exists and toggles dark mode", async ({ page }) => {
  await page.goto("/");

  // The theme toggle button uses a Unicode icon (◐/○/⬤) and has title="Theme: …".
  const themeButton = page.locator("button[title^='Theme:']");
  await expect(themeButton).toBeVisible();

  // Default theme is System — no dark class initially in Playwright Chromium.

  // Click to switch to Light mode.
  await expect(themeButton).toHaveAttribute("title", "Theme: System");
  await themeButton.click();
  await page.waitForTimeout(300);
  await expect(themeButton).toHaveAttribute("title", "Theme: Light");
  await expect(page.locator("html")).not.toHaveClass(/dark/);

  // Click again to switch to Dark mode.
  await themeButton.click();
  await page.waitForTimeout(300);
  await expect(themeButton).toHaveAttribute("title", "Theme: Dark");
  await expect(page.locator("html")).toHaveClass(/dark/);

  // Click again to cycle back to System.
  await themeButton.click();
  await page.waitForTimeout(300);
  await expect(themeButton).toHaveAttribute("title", "Theme: System");
});

test("device dropdown is present in top bar", async ({ page }) => {
  await page.goto("/");

  // The top bar should be visible (h-8 bar at the top).
  // Look for the device dropdown trigger area.
  const topBar = page.locator(".h-8");
  await expect(topBar).toBeVisible();

  // There should be something actionable in the top bar for device selection.
  // The dropdown usually has a button or clickable area.
  const deviceArea = topBar.locator("button, [role='button']").first();
  await expect(deviceArea).toBeVisible();
});
