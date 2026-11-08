"""Render the real UI with an offline ZeroTier fixture at Pixel viewport sizes."""
from pathlib import Path
from playwright.sync_api import sync_playwright
from test_zerotier import ZeroTierHTTP

ZeroTierHTTP.setUpClass()
try:
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width in [360, 412, 448, 1280]:
            page = browser.new_page(viewport={"width": width, "height": 920}, device_scale_factor=1)
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(ZeroTierHTTP.base + "/zerotier", wait_until="networkidle")
            assert not errors, errors
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), width
            assert page.locator('.zt-card').count() == 3
            if width < 768:
                page.locator('.mobile-nav > summary').click()
                link = page.locator('.mobile-nav a[href="/zerotier"]')
                assert link.is_visible()
                assert link.bounding_box()['height'] >= 44
                page.locator('.mobile-nav > summary').click()
            Path('test-artifacts').mkdir(exist_ok=True)
            page.screenshot(path=f'test-artifacts/zerotier-{width}.png', full_page=True)
            page.close()
        browser.close()
finally:
    ZeroTierHTTP.tearDownClass()
