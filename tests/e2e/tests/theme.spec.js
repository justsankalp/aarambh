import { expect, test } from "@playwright/test";
import { mockGitHub } from "./mock-github.js";

// With the wasm blocked, only the inline script in index.html can have set
// the theme, which proves it is applied before the app loads (no flash).
async function blockApp(page) {
  await page.route("**/*.wasm", (route) => route.abort());
}

async function themeOf(page) {
  return page.evaluate(() => ({
    attr: document.documentElement.dataset.theme,
    bg: getComputedStyle(document.body).backgroundColor,
  }));
}

const PAPER = "rgb(244, 241, 234)";
const NIGHT_MOSS = "rgb(18, 22, 20)";

test("auto follows the system theme before the app loads", async ({ page }) => {
  await blockApp(page);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("./");
  expect(await themeOf(page)).toEqual({ attr: "dark", bg: NIGHT_MOSS });
  // The system theme changes while the page is open.
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => themeOf(page)).toEqual({ attr: "light", bg: PAPER });
});

test("a saved choice wins over the system theme before the app loads", async ({ page }) => {
  await blockApp(page);
  await page.addInitScript(() => localStorage.setItem("aarambh.theme", "light"));
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("./");
  expect(await themeOf(page)).toEqual({ attr: "light", bg: PAPER });
});

test("switching theme applies at once and persists across reloads", async ({ page }) => {
  await mockGitHub(page);
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("./#settings");
  const picker = page.getByRole("group", { name: "Theme" });
  await expect(picker.getByRole("button", { name: "Auto" })).toHaveAttribute("aria-pressed", "true");

  await picker.getByRole("button", { name: "Dark" }).click();
  await expect(picker.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true");
  expect(await themeOf(page)).toEqual({ attr: "dark", bg: NIGHT_MOSS });
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute("content", "#121614");

  await blockApp(page);
  await page.reload();
  expect(await themeOf(page)).toEqual({ attr: "dark", bg: NIGHT_MOSS });
});

test("Auto clears the saved choice", async ({ page }) => {
  await mockGitHub(page);
  await page.addInitScript(() => {
    if (!sessionStorage.getItem("seeded")) {
      localStorage.setItem("aarambh.theme", "dark");
      sessionStorage.setItem("seeded", "1");
    }
  });
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("./#settings");
  await page.getByRole("group", { name: "Theme" }).getByRole("button", { name: "Auto" }).click();
  expect(await page.evaluate(() => localStorage.getItem("aarambh.theme"))).toBeNull();
  expect((await themeOf(page)).attr).toBe("light");
});
