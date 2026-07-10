"""E2E test: verify tab switch clears waveform canvases.

Reproduces:
  1. Connect demo device, record data in Tab 1
  2. Click "+" to create Tab 2 (same device, copied config)
  3. Verify Tab 2's canvas pixels are all blank (no stale data from Tab 1)

Root cause: dioxus::document::eval ran during render phase (before DOM commit),
so canvas JS referenced IDs that didn't exist yet.  Fix: moved canvas drawing
into a use_effect hook that runs after DOM commit.
"""

from __future__ import annotations

from playwright.sync_api import Page, expect


def go_home(page: Page) -> None:
    page.goto("/")
    page.wait_for_selector("#main", state="visible")


def canvas_is_blank(page: Page, canvas_selector: str) -> bool | None:
    """Check if a canvas element has only uniform pixels (all same color)."""
    return page.evaluate(
        """([selector]) => {
            const c = document.querySelector(selector);
            if (!c) return null;
            const ctx = c.getContext('2d');
            if (!ctx) return null;
            if (c.width === 0 || c.height === 0) return true;
            const img = ctx.getImageData(0, 0, c.width, c.height);
            const data = img.data;
            if (data.length === 0) return true;
            const r0 = data[0], g0 = data[1], b0 = data[2], a0 = data[3];
            for (let i = 4; i < data.length; i += 4) {
                if (data[i] !== r0 || data[i+1] !== g0 ||
                    data[i+2] !== b0 || data[i+3] !== a0) {
                    return false;
                }
            }
            return true;
        }""",
        [canvas_selector],
    )


def any_canvas_has_signal(page: Page, tab_id: int) -> bool:
    """Return True if any canvas in the given tab shows non-uniform pixels."""
    canvases = page.locator(f'[id^="sig-tab-{tab_id}-"]')
    count = canvases.count()
    for i in range(count):
        canvas_id = canvases.nth(i).get_attribute("id")
        if canvas_id and not canvas_is_blank(page, f"#{canvas_id}"):
            return True
    return False


def test_new_tab_has_blank_waveform(page: Page) -> None:
    """Create Tab 2 after recording in Tab 1; Tab 2's canvases must be blank."""
    go_home(page)

    # ── Check if waveform is already visible (reusing existing session) ──
    if not bool(page.locator('[id^="sig-tab-1-"]').count()):
        # Need to set up: scan → connect demo → record → stop
        scan_btn = page.get_by_role("button", name="Scan for devices")
        if scan_btn.is_visible():
            scan_btn.click()
            page.wait_for_timeout(1000)

        page.locator(".h-8 button").first.click()
        page.wait_for_timeout(300)

        demo_row = page.get_by_text("RustyBench Demo Device").first
        expect(demo_row).to_be_visible(timeout=10_000)
        demo_row.click()
        page.wait_for_timeout(100)

        page.wait_for_selector('[id^="sig-tab-1-"]', state="attached", timeout=15_000)
        page.wait_for_timeout(500)

        run_btn = page.locator("button[title='Start acquisition']")
        expect(run_btn).to_be_visible(timeout=5_000)
        run_btn.click()
        page.wait_for_timeout(200)

        stop_btn = page.locator("button[title='Stop acquisition (device stays connected)']")
        expect(stop_btn).to_be_visible(timeout=5_000)
        stop_btn.click()
        page.wait_for_timeout(500)

    assert page.locator('[id^="sig-tab-1-"]').count() > 0, "Tab 1 must have canvases"

    # ── Verify Tab 1 shows signal data ──────────────────────────────────
    assert any_canvas_has_signal(page, 1), (
        "Tab 1 should have waveform signal data after acquisition"
    )

    # ── Create Tab 2 via "+" button ─────────────────────────────────────
    new_tab_btn = page.locator("button[title='New tab']")
    expect(new_tab_btn).to_be_visible(timeout=5_000)
    new_tab_btn.click()
    page.wait_for_timeout(1000)

    # ── Wait for Tab 2's waveform canvases to render ────────────────────
    page.wait_for_selector('[id^="sig-tab-2-"]', state="attached", timeout=10_000)
    page.wait_for_timeout(1000)

    # ── Check: Tab 2's canvases should all be blank ────────────────────
    assert not any_canvas_has_signal(page, 2), (
        "Tab 2's waveform canvases must be blank (no stale data from Tab 1)"
    )
