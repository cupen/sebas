"""Fake Anthropic Messages provider for smoke-testing agent-dev --shell.

Listens on 127.0.0.1:PORT (argv[1]). For every POST /v1/messages it answers
with a tiny SSE stream whose text echoes the user's first prompt word, so the
shell round-trip can be asserted. Test-only scaffolding.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


def sse_frame(event: str, data: dict) -> bytes:
    return f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()


def sse_turn(text: str) -> bytes:
    body = b""
    body += sse_frame("message_start", {"type": "message_start", "message": {"id": "msg_fake"}})
    body += sse_frame("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text"}})
    body += sse_frame("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}})
    body += sse_frame("content_block_stop", {"type": "content_block_stop", "index": 0})
    body += sse_frame("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}})
    body += sse_frame("message_stop", {"type": "message_stop"})
    return body


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        payload = json.loads(self.rfile.read(length) or b"{}")
        # 取用户 prompt 的最后一个词作为回复（可断言请求确实带上了 prompt）
        words = []
        for msg in payload.get("messages", []):
            for block in msg.get("content", []):
                if isinstance(block, dict) and block.get("type") == "text":
                    words = block.get("text", "").split()
        last = words[-1] if words else "empty"
        # prompt 含 "sleep" → 回一个 bash sleep 60 工具调用（测 /cancel 交互）
        if any("sleep" in w for w in words):
            body = b""
            body += sse_frame("message_start", {"type": "message_start", "message": {"id": "msg_fake"}})
            body += sse_frame("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "bash", "input": {}}})
            body += sse_frame("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps({"command": "sleep 60"})}})
            body += sse_frame("content_block_stop", {"type": "content_block_stop", "index": 0})
            body += sse_frame("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use"}})
            body += sse_frame("message_stop", {"type": "message_stop"})
        else:
            body = sse_turn(f"fake-reply-for-{last}")
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1])
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
