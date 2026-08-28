#!/usr/bin/env python3
"""One-shot Selenium bridge used by drm-exec's web.selenium capability."""

import argparse
import ipaddress
import json
import os
import socket
import sys
from urllib.parse import urlparse

from selenium import webdriver
from selenium.webdriver.chrome.options import Options


def reject_private_resolution(url: str) -> None:
    if os.environ.get("DRMD_WEB_ALLOW_PRIVATE") == "1":
        return
    host = urlparse(url).hostname
    if not host:
        raise ValueError("URL has no hostname")
    for result in socket.getaddrinfo(host, None):
        address = ipaddress.ip_address(result[4][0])
        if not address.is_global:
            raise ValueError(f"hostname {host!r} resolves to blocked address {address}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--application-id", required=True)
    parser.add_argument("--webdriver-url")
    parser.add_argument("--timeout", type=int, default=20)
    parser.add_argument("--max-output-bytes", type=int, default=1_000_000)
    args = parser.parse_args()

    options = Options()
    for flag in ("--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"):
        options.add_argument(flag)
    driver = None
    try:
        reject_private_resolution(args.url)
        if args.webdriver_url:
            driver = webdriver.Remote(command_executor=args.webdriver_url, options=options)
        else:
            driver = webdriver.Chrome(options=options)
        driver.set_page_load_timeout(args.timeout)
        driver.get(args.url)
        reject_private_resolution(driver.current_url)
        text = driver.find_element("tag name", "body").text
        payload = {
            "application_id": args.application_id,
            "final_url": driver.current_url,
            "title": driver.title,
            "text": text,
        }
        encoded = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        if len(encoded) > args.max_output_bytes:
            # Preserve valid JSON while reducing the only unbounded field.
            low, high = 0, len(text)
            while low < high:
                mid = (low + high + 1) // 2
                payload["text"] = text[:mid]
                if len(json.dumps(payload, ensure_ascii=False).encode("utf-8")) <= args.max_output_bytes:
                    low = mid
                else:
                    high = mid - 1
            payload["text"] = text[:low]
            payload["truncated"] = True
            encoded = json.dumps(payload, ensure_ascii=False).encode("utf-8")
            if len(encoded) > args.max_output_bytes:
                raise ValueError("max output is too small for metadata")
        sys.stdout.buffer.write(encoded)
        return 0
    except Exception as exc:  # Selenium exposes multiple backend-specific exception types.
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    finally:
        if driver is not None:
            driver.quit()


if __name__ == "__main__":
    raise SystemExit(main())
