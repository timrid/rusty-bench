"""E2E tests for the waveform view: row resize, labels, and canvas layout."""

import re

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def go_home(page: Page) -> None:
    page.goto("/")
    page.wait_for_selector("#main", state="visible")


def waveform_is_visible(page: Page) -> bool:
    """Return True when at least one waveform row canvas is rendered.

    The waveform area only appears when a device is connected and acquisition
    data is available.  In the "No Device" idle state this returns False.
    """
    # The right panel contains row canvases; its id is rows-tab-<N>.
    rows_panel = page.locator('[id^="rows-tab-"]')
    if not rows_panel.count():
        return False
    # Inside the rows panel, each signal row has its own <canvas>.
    canvases = rows_panel.first.locator("canvas")
    return canvases.count() > 0


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

def test_waveform_view_renders_when_data_is_present(
    page: Page, web_server: str
) -> None:
    """The waveform canvas area and label panel appear when data is loaded."""
    go_home(page)

    if not waveform_is_visible(page):
        pytest.skip("No waveform data available – connect a device first.")

    labels = page.locator('[id^="labels-tab-"]').first
    rows = page.locator('[id^="rows-tab-"]').first

    expect(labels).to_be_visible()
    expect(rows).to_be_visible()
    # At least one canvas per visible row
    expect(rows.locator("canvas").first).to_be_visible()


def test_row_dividers_exist_between_rows(page: Page, web_server: str) -> None:
    """Every pair of adjacent visible waveform rows has a resize divider."""
    go_home(page)

    if not waveform_is_visible(page):
        pytest.skip("No waveform data available – connect a device first.")

    dividers = get_row_dividers(page)
    visible_rows = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )

    # N rows → N dividers (one after each row)
    row_count = visible_rows.count()
    assert row_count > 0, "Expected at least one visible waveform row"
    assert len(dividers) == row_count, (
        f"Expected {row_count} dividers (one after each row), got {len(dividers)}"
    )


def test_resize_divider_has_correct_cursor_and_structure(
    page: Page, web_server: str
) -> None:
    """Each row divider is a narrow bar with the ns-resize cursor."""
    go_home(page)

    if not waveform_is_visible(page):
        pytest.skip("No waveform data available – connect a device first.")

    dividers = get_row_dividers(page)
    assert len(dividers) > 0, "Expected at least one row divider"

    for i, handle in enumerate(dividers):
        expect(handle).to_be_visible()
        # The handle itself has cursor-ns-resize (set on the div)
        classes = handle.get_attribute("class") or ""
        assert "cursor-ns-resize" in classes, (
            f"Divider {i} missing cursor-ns-resize class"
        )


def test_row_resize_changes_row_height(page: Page, web_server: str) -> None:
    """Dragging a row divider vertically changes the row's height.

    This test performs a real mouse-drag interaction on the first row divider.
    It requires ``--headed`` because Playwright needs a visible viewport to
    compute element bounding boxes correctly.
    """
    go_home(page)

    if not waveform_is_visible(page):
        pytest.skip("No waveform data available – connect a device first.")

    dividers = get_row_dividers(page)
    assert len(dividers) >= 1, "Need at least one divider for resize test"

    # Grab the first row label (the row *above* the first divider)
    row_labels = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )
    first_row = row_labels.first
    initial_box = first_row.bounding_box()
    assert initial_box is not None, "Could not get bounding box of first row"
    initial_height = initial_box["height"]

    # Drag the divider down by 30 px
    divider = dividers[0]
    divider_box = divider.bounding_box()
    assert divider_box is not None, "Could not get bounding box of divider"

    start_x = divider_box["x"] + divider_box["width"] / 2
    start_y = divider_box["y"] + divider_box["height"] / 2

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y + 30, steps=5)
    page.mouse.up()

    # Allow Dioxus to process the state update
    page.wait_for_timeout(200)

    new_box = first_row.bounding_box()
    assert new_box is not None, "Row disappeared after resize"
    new_height = new_box["height"]

    assert new_height > initial_height, (
        f"Expected row height to increase after dragging down. "
        f"Was {initial_height:.1f} px, now {new_height:.1f} px"
    )


def test_row_height_never_below_minimum(page: Page, web_server: str) -> None:
    """Dragging a divider far upward clamps the row height to ≥ 10 px."""
    go_home(page)

    if not waveform_is_visible(page):
        pytest.skip("No waveform data available – connect a device first.")

    dividers = get_row_dividers(page)
    assert len(dividers) >= 1, "Need at least one divider for resize test"

    # Use the last divider and the last row (above it)
    # The very last divider sits after the last row and can shrink it.
    last_divider = dividers[-1]
    row_labels = (
        page.locator('[id^="labels-tab-"]').first.locator("div.cursor-grab")
    )
    last_row = row_labels.last

    divider_box = last_divider.bounding_box()
    assert divider_box is not None

    start_x = divider_box["x"] + divider_box["width"] / 2
    start_y = divider_box["y"] + divider_box["height"] / 2

    # Drag far upward (way more than any reasonable row height)
    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x, start_y - 200, steps=5)
    page.mouse.up()

    page.wait_for_timeout(200)

    new_box = last_row.bounding_box()
    assert new_box is not None, "Row disappeared after upward resize"
    assert new_box["height"] >= 10.0, (
        f"Row height should be clamped to ≥ 10 px, got {new_box['height']:.1f} px"
    )
