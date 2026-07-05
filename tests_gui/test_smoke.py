"""Smoke / sanity tests for the rb-gui web frontend."""

import re

from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def go_home(page: Page) -> None:
    page.goto("/")
    # Wait until Dioxus rendered the app shell.
    page.wait_for_selector("#main", state="visible")


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_app_mounts_and_shows_branding(page: Page) -> None:
    """The main mount point is visible and the page title is correct."""
    go_home(page)

    main = page.locator("#main")
    expect(main).to_be_visible()
    expect(page).to_have_title(re.compile(r"^RustyBench"))
    expect(main).to_contain_text("Idle")


def test_shows_no_device_placeholder(page: Page) -> None:
    """When no device is connected the UI tells the user."""
    go_home(page)

    expect(page.locator("#main")).to_contain_text("No Device")
    expect(
        page.get_by_role("button", name=re.compile("scan for devices", re.IGNORECASE))
    ).to_be_visible()


def test_status_bar_shows_application_version(page: Page) -> None:
    """The status bar includes the version string."""
    go_home(page)

    expect(page.locator("#main")).to_contain_text("RustyBench v0.3.0")


def test_theme_toggle_exists_and_toggles_dark_mode(page: Page) -> None:
    """Clicking the theme button cycles System → Light → Dark → System."""
    go_home(page)

    theme_button = page.locator("button[title^='Theme:']")
    expect(theme_button).to_be_visible()
    expect(theme_button).to_have_attribute("title", "Theme: System")

    # System → Light
    theme_button.click()
    page.wait_for_timeout(300)
    expect(theme_button).to_have_attribute("title", "Theme: Light")
    expect(page.locator("html")).not_to_have_class(re.compile("dark"))

    # Light → Dark
    theme_button.click()
    page.wait_for_timeout(300)
    expect(theme_button).to_have_attribute("title", "Theme: Dark")
    expect(page.locator("html")).to_have_class(re.compile("dark"))

    # Dark → System
    theme_button.click()
    page.wait_for_timeout(300)
    expect(theme_button).to_have_attribute("title", "Theme: System")


def test_device_dropdown_is_present_in_top_bar(page: Page) -> None:
    """The top bar contains a clickable device selection area."""
    go_home(page)

    top_bar = page.locator(".h-8")
    expect(top_bar).to_be_visible()

    device_area = top_bar.locator("button, [role='button']").first
    expect(device_area).to_be_visible()
