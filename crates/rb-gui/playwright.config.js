// @ts-check
const { defineConfig, devices } = require("@playwright/test");

module.exports = defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL: "http://127.0.0.1:9990",
    trace: "retain-on-failure",
    navigationTimeout: 10 * 60 * 1000,
  },
  timeout: 10 * 60 * 1000,

  webServer: [
    {
      command:
        process.platform === "win32"
          ? "dx serve --platform web --port 9990"
          : "dx serve --platform web --port 9990",
      port: 9990,
      timeout: 10 * 60 * 1000,
      reuseExistingServer: !process.env.CI,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
