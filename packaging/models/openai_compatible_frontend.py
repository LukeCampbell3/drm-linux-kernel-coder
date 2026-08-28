#!/usr/bin/env python3
"""Loopback-only adapter for locally served GLM and Qwen checkpoints."""
import argparse
import json
import sys
import urllib.request
from urllib.parse import urlparse

PROVIDERS = {
    "glm": ("http://127.0.0.1:8000/v1/chat/completions", "zai-org/GLM-5.3-Flash"),
    "qwen": ("http://127.0.0.1:8001/v1/chat/completions", "Qwen/Qwen3-Coder-Next"),
}
SYSTEM = """You are an intent planner, never an application operator. Return exactly four lines:
decision=watch|execute|clarify|deny
family=a short snake_case identifier
capability=task.watch|app.execute|web.selenium|code.evolve
confidence_milli=0..1000
Use execute/app.execute only when the user explicitly says the workflow is already DRM-certified. Use watch/task.watch for unfamiliar application workflows. Use clarify for ambiguity or consequential actions. Deny requests to evade safeguards. Never output actions, commands, credentials, URLs, or prose."""

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", choices=PROVIDERS, required=True)
    parser.add_argument("--goal", required=True)
    args = parser.parse_args()
    url, model = PROVIDERS[args.provider]
    parsed = urlparse(url)
    if parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "::1", "localhost"}:
        sys.exit("model endpoint must be loopback HTTP")
    body = json.dumps({
        "model": model,
        "temperature": 0,
        "max_tokens": 160,
        "reasoning_effort": "low",
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": args.goal}],
    }).encode()
    request = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.load(response)
    content = payload["choices"][0]["message"]["content"]
    if not isinstance(content, str):
        raise ValueError("model returned non-text content")
    print(content.strip())

if __name__ == "__main__":
    main()
