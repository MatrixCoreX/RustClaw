import json
import sys


def respond(request: dict) -> dict:
    return {
        "request_id": request["request_id"],
        "status": "ok",
        "text": "",
        "error_text": None,
        "extra": {"result": {"handled": True}},
    }


def main() -> None:
    line = sys.stdin.buffer.readline()
    try:
        request = json.loads(line)
        response = respond(request)
    except Exception as error:  # protocol boundary
        response = {
            "request_id": "invalid",
            "status": "error",
            "text": "",
            "error_text": str(error),
            "extra": {
                "error_code": "request_invalid",
                "message_key": "skill.request_invalid",
            },
        }
    sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
