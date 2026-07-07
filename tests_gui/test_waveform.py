"""E2E tests for the waveform view: row resize, labels, and canvas layout.

These tests connect the built-in **demo device**, run a short acquisition,
and then exercise the waveform UI.  No physical hardware is required.
"""

from __future__ import annotations

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


# ---------------------------------------------------------------------------
# Row Reorder helpers
# ---------------------------------------------------------------------------

def get_row_labels(page: Page):
    """Return all visible row label elements (draggable rows).

    Note: during a drag, the source row gets ``invisible`` class but is
    still in the DOM and preserves its space.
    """
    return (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab").all()
    )


def get_amber_indicator(page: Page):
    """Return the amber insertion indicator, if visible.

    The target gap is rendered as an expanded divider (row-height) with
    a dashed amber border.  Look for:
    - A ``div`` inside the labels panel with class ``border-amber-400``
    - A ``div.z-20.pointer-events-none`` (top/bottom edge)
    """
    labels = page.locator('[id^="labels-tab-"]').first

    # Top/bottom edge indicator
    top_edge = labels.locator("div.z-20.pointer-events-none")
    if top_edge.count() > 0:
        return top_edge.first

    # Target gap: dashed border div with amber-400 class
    target_gap = labels.locator("div.border-amber-400")
    if target_gap.count() > 0:
        return target_gap.first

    return None


def get_floating_label(page: Page):
    """Return the floating label clone (position:fixed) if visible.

    The floating label is rendered outside the normal flow with
    ``class="fixed z-50 pointer-events-none"`` and contains a border-amber-400
    div.
    """
    return page.locator("div.fixed.z-50.pointer-events-none").first


def get_row_label_texts(page: Page) -> list[str]:
    """Return the visible text of each row label in top-to-bottom order."""
    labels = get_row_labels(page)
    return [
        (lbl.inner_text() or "").strip() for lbl in labels
    ]


# ---------------------------------------------------------------------------
# Row Reorder Tests (Drag & Drop)
# ---------------------------------------------------------------------------

def test_reorder_drag_shows_floating_label(page: Page) -> None:
    """When a row label is dragged, a floating clone follows the cursor."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2, (
        "Need at least 2 rows to test reorder, "
        f"got {len(labels)}"
    )

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2

    # Start dragging the first label downward
    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y + 60, steps=5)
    page.wait_for_timeout(300)

    # The floating label clone should be visible
    floating = get_floating_label(page)
    assert floating.count() > 0, (
        "Expected floating label clone to be visible during drag"
    )
    expect(floating).to_be_visible()

    # The source row should be invisible (space preserved)
    source = labels[0]
    classes = source.get_attribute("class") or ""
    assert "invisible" in classes, (
        f"Expected source row to be invisible during drag, got: {classes}"
    )

    # Clean up: release the drag
    page.mouse.up()
    page.wait_for_timeout(200)


def test_reorder_drag_shows_target_gap(page: Page) -> None:
    """When dragging a row between two others, an expanded target gap
    appears with a dashed amber border."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2

    # Drag downward to hover between row 1 and row 2
    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y + 80, steps=5)
    page.wait_for_timeout(300)

    # The amber indicator (target gap or line) should appear
    indicator = get_amber_indicator(page)
    assert indicator is not None, (
        "Expected target gap / amber indicator to be visible during drag"
    )

    page.mouse.up()
    page.wait_for_timeout(200)


def test_reorder_drop_moves_row(page: Page) -> None:
    """Dropping a dragged label reorders the rows in the label panel."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2, (
        "Need at least 2 rows to test reorder, "
        f"got {len(labels)}"
    )

    # Record the initial order
    initial_texts = get_row_label_texts(page)
    assert len(initial_texts) >= 2

    # Drag the first label down past the second
    first_label = labels[0]
    second_label = labels[1]

    first_box = first_label.bounding_box()
    second_box = second_label.bounding_box()
    assert first_box is not None
    assert second_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2
    target_y = second_box["y"] + second_box["height"] + 10  # below second row

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, target_y, steps=10)
    page.mouse.up()
    page.wait_for_timeout(300)

    # The first and second labels should have swapped
    new_texts = get_row_label_texts(page)
    assert len(new_texts) >= 2

    assert new_texts[0] == initial_texts[1], (
        f"Expected second row '{initial_texts[1]}' to become first, "
        f"but first is '{new_texts[0]}'. Full order: {new_texts}"
    )
    assert new_texts[1] == initial_texts[0], (
        f"Expected first row '{initial_texts[0]}' to become second, "
        f"but second is '{new_texts[1]}'. Full order: {new_texts}"
    )


def test_reorder_floating_label_disappears_after_mouseup(page: Page) -> None:
    """The floating label and indicator are removed after the drag completes."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2

    # Drag down to trigger the indicator and floating label
    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y + 80, steps=5)
    page.wait_for_timeout(300)

    # Both should be present during drag
    floating = get_floating_label(page)
    assert floating.count() > 0, "Expected floating label during drag"

    indicator = get_amber_indicator(page)
    assert indicator is not None, "Expected indicator during drag"

    # Release the mouse
    page.mouse.up()
    page.wait_for_timeout(300)

    # Floating label should be gone
    floating = get_floating_label(page)
    assert floating.count() == 0, (
        "Floating label should be removed after mouse up"
    )

    # Indicator should be gone
    indicator = get_amber_indicator(page)
    assert indicator is None, (
        "Amber insertion indicator should be removed after mouse up"
    )


def test_reorder_label_not_permanently_hidden(page: Page) -> None:
    """After drag completes (even outside label panel), no label stays
    permanently invisible."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2

    # Drag to the right (into the canvas area) and release there
    page.mouse.move(start_x, start_y)
    page.mouse.down()
    # Move horizontally into the canvas area
    page.mouse.move(start_x + 200, start_y + 200, steps=10)
    page.mouse.up()
    page.wait_for_timeout(300)

    # All visible labels should NOT have invisible class
    for i, lbl in enumerate(get_row_labels(page)):
        classes = lbl.get_attribute("class") or ""
        assert "invisible" not in classes, (
            f"Label {i} still invisible after drag completed. "
            f"Classes: {classes}"
        )


def test_reorder_floating_label_tracks_x(page: Page) -> None:
    """The floating label follows the cursor horizontally, not just vertically.

    Clicks at an offset within the label should make the label stick to
    the cursor at that exact point in both X and Y.
    """
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    # Click near the right edge of the label (offset ~80% of width)
    click_x = first_box["x"] + first_box["width"] * 0.8
    click_y = first_box["y"] + first_box["height"] / 2

    page.mouse.move(click_x, click_y)
    page.mouse.down()

    # Move horizontally only, no vertical change
    target_x = click_x + 50
    page.mouse.move(target_x, click_y, steps=5)
    page.wait_for_timeout(300)

    # The floating label should have moved right by ~50px
    floating = get_floating_label(page)
    assert floating.count() > 0, "Expected floating label during drag"

    floating_box = floating.bounding_box()
    assert floating_box is not None

    # The floating label's left edge should be near target_x - click_offset_x
    # Since click_offset_x ≈ 80% of label width, the label should have moved
    # to the right of its original position.
    assert floating_box["x"] > first_box["x"] + 20, (
        f"Floating label should have moved right. "
        f"Original x={first_box['x']:.0f}, floating x={floating_box['x']:.0f}"
    )

    page.mouse.up()
    page.wait_for_timeout(200)


def test_reorder_floating_label_sticks_at_click_point(page: Page) -> None:
    """When clicking near the bottom of a label, the cursor stays near the
    bottom of the floating label (not snapping to the center)."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    first_label = labels[0]
    first_box = first_label.bounding_box()
    assert first_box is not None

    # Click near the bottom of the label
    click_x = first_box["x"] + first_box["width"] / 2
    click_y = first_box["y"] + first_box["height"] * 0.9  # 90% down

    page.mouse.move(click_x, click_y)
    page.mouse.down()
    page.wait_for_timeout(100)

    # Move down by 40px
    page.mouse.move(click_x, click_y + 40, steps=5)
    page.wait_for_timeout(300)

    floating = get_floating_label(page)
    assert floating.count() > 0

    floating_box = floating.bounding_box()
    assert floating_box is not None

    # The floating label's top should be below the original label's top
    # by roughly 40px (the Y movement), because the click point sticks.
    expected_top = first_box["y"] + 40
    assert abs(floating_box["y"] - expected_top) < 20, (
        f"Floating label Y should follow cursor with click-point offset. "
        f"Expected ~{expected_top:.0f}, got {floating_box['y']:.0f}"
    )

    page.mouse.up()
    page.wait_for_timeout(200)


def test_reorder_target_does_not_flicker(page: Page) -> None:
    """The target insertion position should be stable when the cursor moves
    smoothly past a divider — no jumping back and forth between two
    different non-source positions."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 3, (
        "Need at least 3 rows, got {len(labels)}"
    )

    # Drag row 0 so cursor passes row 1 and enters row 2 area.
    first_label = labels[0]
    second_label = labels[1]
    third_label = labels[2]

    first_box = first_label.bounding_box()
    second_box = second_label.bounding_box()
    third_box = third_label.bounding_box()
    assert first_box is not None and second_box is not None and third_box is not None

    start_x = first_box["x"] + first_box["width"] / 2
    start_y = first_box["y"] + first_box["height"] / 2

    page.mouse.move(start_x, start_y)
    page.mouse.down()

    # Move down in small increments from row 1 area into row 2 area.
    # We track the indicator Y; rapid back-and-forth between two distinct
    # positions (not just appear/disappear) counts as flicker.
    prev_y: float | None = None
    y_changes = 0
    for step in range(10, 200, 10):
        page.mouse.move(start_x, start_y + step, steps=1)
        page.wait_for_timeout(30)

        indicator = get_amber_indicator(page)
        if indicator is not None:
            box = indicator.bounding_box()
            if box is not None:
                y = box["y"]
                if prev_y is not None and abs(y - prev_y) > 3:
                    y_changes += 1
                prev_y = y

    page.mouse.up()
    page.wait_for_timeout(200)

    # With smooth cursor motion, the indicator position should change at most
    # a few times (once per row transition).  More than 5 distinct positions
    # would indicate rapid flickering.
    assert y_changes <= 5, (
        f"Indicator position changed {y_changes} times — expected ≤ 5 for smooth "
        f"transitions across 2 rows"
    )


def test_reorder_insert_at_top(page: Page) -> None:
    """Dragging the last row to the very top shows the top-edge indicator
    and reorders correctly."""
    ensure_waveform_visible(page)

    labels = get_row_labels(page)
    assert len(labels) >= 2

    initial_texts = get_row_label_texts(page)
    last_label = labels[-1]
    first_label = labels[0]

    last_box = last_label.bounding_box()
    first_box = first_label.bounding_box()
    assert last_box is not None
    assert first_box is not None

    start_x = last_box["x"] + last_box["width"] / 2
    start_y = last_box["y"] + last_box["height"] / 2
    # Target just inside the first row (the body div must still capture
    # onmousemove events).  compute_reorder_target will find the nearest
    # gap — which is position 0 (above row 0).
    target_y = first_box["y"] + 5

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, target_y, steps=10)
    page.wait_for_timeout(300)

    # Top-edge indicator should be visible (expanded gap with dashed border)
    indicator = get_amber_indicator(page)
    assert indicator is not None, (
        "Expected top-edge amber indicator when dragging above first row. "
        f"first_box.y={first_box['y']:.0f}, target_y={target_y:.0f}"
    )

    page.mouse.up()
    page.wait_for_timeout(300)

    new_texts = get_row_label_texts(page)
    # Last row should now be first
    assert new_texts[0] == initial_texts[-1], (
        f"Expected last row '{initial_texts[-1]}' to move to top. "
        f"Order: {new_texts}"
    )
