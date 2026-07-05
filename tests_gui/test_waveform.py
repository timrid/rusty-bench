"""E2E tests for the waveform view: row resize, labels, and canvas layout.

These tests connect the built-in **demo device**, run a short acquisition,
and then exercise the waveform UI.  No physical hardware is required.
"""

from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def go_home(page: Page) -> None:
    page.goto("/")
    page.wait_for_selector("#main", state="visible")


def waveform_is_visible(page: Page) -> bool:
    """Return ``True`` when at least one waveform row canvas is rendered."""
    rows_panel = page.locator('[id^="rows-tab-"]')
    if not rows_panel.count():
        return False
    canvases = rows_panel.first.locator("canvas")
    return canvases.count() > 0


def ensure_waveform_visible(page: Page) -> None:
    """Make sure the waveform view is populated with acquisition data.

    Connects the built-in demo device, starts a short acquisition, and
    stops it so that waveform rows and canvases are rendered.  Fails the
    test if any step cannot be completed.
    """
    go_home(page)

    if waveform_is_visible(page):
        return

    # ── 1. Open the device dropdown in the top bar ────────────────────
    page.locator(".h-8 button").first.click()
    page.wait_for_timeout(300)

    # ── 2. Click the "RustyBench Demo Device" row ─────────────────────
    # dispatch_event("click") fires the native event and returns
    # immediately — essential because Dioxus removes the dropdown row
    # from the DOM during the async connect handler.
    demo_row = page.get_by_text("RustyBench Demo Device").first
    expect(demo_row).to_be_visible(timeout=10_000)

    # Capture console errors during the connect attempt.
    console_msgs: list[str] = []
    page.on("console", lambda msg: console_msgs.append(f"[{msg.type}] {msg.text}"))

    demo_row.click()
    page.wait_for_timeout(100)

    # ── 3. Wait until the waveform view replaces "No Device" ──────────
    try:
        page.wait_for_selector(
            '[id^="rows-tab-"]', state="visible", timeout=15_000
        )
    except Exception:
        errors = "\n".join(console_msgs[-20:]) if console_msgs else "(none)"
        raise AssertionError(
            "Waveform view did not appear after demo device connect.\n"
            "The demo device may not be available in this build, or the\n"
            "async connect handler failed silently.\n\n"
            f"Browser console (last 20 messages):\n{errors}"
        )
    page.wait_for_timeout(100)

    # ── 4. Start acquisition ─────────────────────────────────────────
    run_button = page.locator("button[title='Start acquisition']")
    expect(run_button).to_be_visible(timeout=5_000)
    run_button.click()

    # ── 5. Let samples accumulate ─────────────────────────────────────
    page.wait_for_timeout(100)

    # ── 6. Stop acquisition ──────────────────────────────────────────
    stop_button = page.locator(
        "button[title='Stop acquisition (device stays connected)']"
    )
    expect(stop_button).to_be_visible(timeout=5_000)
    stop_button.click()
    page.wait_for_timeout(500)

    # ── 7. Sanity check ───────────────────────────────────────────────
    assert waveform_is_visible(page), (
        "Waveform view did not appear after demo acquisition setup"
    )


def get_row_dividers(page: Page) -> list:
    """Return all resize-handle elements (row-height dividers).

    Dividers live inside the labels panel and carry the ``cursor-ns-resize``
    class.  Each divider sits between two visible waveform rows.
    """
    labels_panel = page.locator('[id^="labels-tab-"]').first
    return labels_panel.locator("div.cursor-ns-resize").all()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_waveform_view_renders_when_data_is_present(page: Page) -> None:
    """The waveform canvas area and label panel appear when data is loaded."""
    ensure_waveform_visible(page)

    labels = page.locator('[id^="labels-tab-"]').first
    rows = page.locator('[id^="rows-tab-"]').first

    expect(labels).to_be_visible()
    expect(rows).to_be_visible()
    expect(rows.locator("canvas").first).to_be_visible()


def test_row_dividers_exist_between_rows(page: Page) -> None:
    """Every visible waveform row has a trailing resize divider."""
    ensure_waveform_visible(page)

    dividers = get_row_dividers(page)
    visible_rows = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )

    row_count = visible_rows.count()
    assert row_count > 0, "Expected at least one visible waveform row"
    assert len(dividers) == row_count, (
        f"Expected {row_count} dividers, got {len(dividers)}"
    )


def test_resize_divider_has_correct_cursor_and_structure(page: Page) -> None:
    """Each row divider is visible with the ns-resize cursor class."""
    ensure_waveform_visible(page)

    dividers = get_row_dividers(page)
    assert len(dividers) > 0, "Expected at least one row divider"

    for i, handle in enumerate(dividers):
        expect(handle).to_be_visible()
        classes = handle.get_attribute("class") or ""
        assert "cursor-ns-resize" in classes, (
            f"Divider {i} missing cursor-ns-resize class"
        )


def test_row_resize_changes_row_height(page: Page) -> None:
    """Dragging a row divider down increases the row above it.

    Requires ``--headed`` for reliable bounding-box computation.
    """
    ensure_waveform_visible(page)

    dividers = get_row_dividers(page)
    assert len(dividers) >= 1, "Need at least one divider"

    row_labels = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )
    first_row = row_labels.first
    initial_box = first_row.bounding_box()
    assert initial_box is not None, "Could not get bounding box of first row"
    initial_height = initial_box["height"]

    divider = dividers[0]
    divider_box = divider.bounding_box()
    assert divider_box is not None

    start_x = divider_box["x"] + divider_box["width"] / 2
    start_y = divider_box["y"] + divider_box["height"] / 2

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y + 30, steps=5)
    page.mouse.up()

    page.wait_for_timeout(200)

    new_box = first_row.bounding_box()
    assert new_box is not None, "Row disappeared after resize"
    new_height = new_box["height"]

    assert new_height > initial_height, (
        f"Row height did not increase: {initial_height:.1f} → {new_height:.1f} px"
    )


def test_row_height_never_below_minimum(page: Page) -> None:
    """Dragging a divider far upward clamps the row height to ≥ 10 px."""
    ensure_waveform_visible(page)

    dividers = get_row_dividers(page)
    assert len(dividers) >= 1, "Need at least one divider"

    last_divider = dividers[-1]
    row_labels = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )
    last_row = row_labels.last

    divider_box = last_divider.bounding_box()
    assert divider_box is not None

    start_x = divider_box["x"] + divider_box["width"] / 2
    start_y = divider_box["y"] + divider_box["height"] / 2

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y - 200, steps=5)
    page.mouse.up()

    page.wait_for_timeout(200)

    new_box = last_row.bounding_box()
    assert new_box is not None, "Row disappeared after upward resize"
    assert new_box["height"] >= 10.0, (
        f"Row height below minimum: {new_box['height']:.1f} px (expected ≥ 10)"
    )
