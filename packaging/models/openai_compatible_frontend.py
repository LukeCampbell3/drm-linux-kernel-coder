#!/usr/bin/env python3
"""OpenAI-compatible intent adapter for GLM-5.3-Flash and Qwen3.8-Flash."""
import argparse
import json
import os
import sys
import urllib.request

PROVIDERS = {
    "glm": ("ZAI_API_KEY", "https://api.z.ai/api/paas/v4/chat/completions", "glm-5.3-flash"),
    "qwen": ("DASHSCOPE_API_KEY", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions", "qwen3.8-flash"),
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
    key_name, default_url, default_model = PROVIDERS[args.provider]
    api_key = os.environ.get(key_name)
    if not api_key:
        sys.exit(f"missing {key_name}")
    prefix = "DRMD_GLM" if args.provider == "glm" else "DRMD_QWEN"
    url = os.environ.get(prefix + "_BASE_URL", default_url)
    model = os.environ.get(prefix + "_MODEL", default_model)
    body = json.dumps({
        "model": model,
        "temperature": 0,
        "max_tokens": 160,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": args.goal}],
    }).encode()
    request = urllib.request.Request(url, data=body, headers={"Authorization": "Bearer " + api_key, "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.load(response)
    content = payload["choices"][0]["message"]["content"]
    if not isinstance(content, str):
        raise ValueError("model returned non-text content")
    print(content.strip())

if __name__ == "__main__":
    main()
