"""Start splitrail's Ollama usage proxy on a Windows GPU box (windowless, via pythonw).

Clients on the LAN keep calling :11434; Ollama itself moves to 127.0.0.1:11435
(OLLAMA_HOST). Usage goes to %USERPROFILE%\\.config\\splitrail\\ollama\\usage.jsonl, which the
publisher reads; GPU power comes from nvidia-smi. Installed by install_windows_publisher.ps1.
"""
import os
import sys

os.environ.setdefault("OLLAMA_PROXY_HOST", "0.0.0.0")
os.environ.setdefault("OLLAMA_PROXY_PORT", "11434")
os.environ.setdefault("OLLAMA_UPSTREAM", "http://127.0.0.1:11435")
here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, here)
log = open(os.path.join(os.path.expanduser("~"), ".splitrail.log"), "a", buffering=1)
sys.stdout = sys.stderr = log  # pythonw has no console.

import ollama_usage_proxy  # noqa: E402

sys.exit(ollama_usage_proxy.main())
