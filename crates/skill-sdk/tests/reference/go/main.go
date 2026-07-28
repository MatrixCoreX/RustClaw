package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"
)

type request struct {
	RequestID string         `json:"request_id"`
	Args      map[string]any `json:"args"`
}

func ok(requestID string, extra map[string]any) map[string]any {
	return map[string]any{"request_id": requestID, "status": "ok", "text": "", "error_text": nil, "extra": extra}
}

func main() {
	var input request
	if err := json.NewDecoder(bufio.NewReader(os.Stdin)).Decode(&input); err != nil {
		return
	}
	action, _ := input.Args["action"].(string)
	if action == "" {
		action = "calculate"
	}
	var output map[string]any
	switch action {
	case "calculate":
		a, _ := input.Args["a"].(float64)
		b, _ := input.Args["b"].(float64)
		output = ok(input.RequestID, map[string]any{"result": map[string]any{"value": a + b}})
	case "validation_error":
		output = map[string]any{"request_id": input.RequestID, "status": "error", "text": "", "error_text": "invalid fixture input", "extra": map[string]any{"error_code": "fixture_invalid", "message_key": "fixture.invalid"}}
	case "artifact":
		path, _ := input.Args["artifact_path"].(string)
		_ = os.WriteFile(path, []byte("reference-artifact\n"), 0o600)
		output = ok(input.RequestID, map[string]any{"artifact": map[string]any{"created": true}})
	case "waiting":
		output = ok(input.RequestID, map[string]any{"continuation": map[string]any{"state": "waiting", "poll_after_ms": 10}})
	case "needs_user":
		output = ok(input.RequestID, map[string]any{"continuation": map[string]any{"state": "needs_user", "required_fields": []string{"confirmation"}}})
	case "timeout":
		time.Sleep(5 * time.Second)
		output = ok(input.RequestID, map[string]any{})
	case "malformed":
		fmt.Println("{not-json")
		return
	case "multiple":
		bytes, _ := json.Marshal(ok(input.RequestID, map[string]any{}))
		fmt.Println(string(bytes))
		fmt.Println(string(bytes))
		return
	case "oversized":
		fmt.Println(strings.Repeat("x", 1024*1024+1))
		return
	case "stderr":
		fmt.Fprintln(os.Stderr, "reference diagnostic")
		output = ok(input.RequestID, map[string]any{"diagnostic_preserved": true})
	default:
		output = ok(input.RequestID, map[string]any{})
	}
	_ = json.NewEncoder(os.Stdout).Encode(output)
}
