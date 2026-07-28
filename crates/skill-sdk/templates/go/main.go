package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

type request struct {
	RequestID string         `json:"request_id"`
	Args      map[string]any `json:"args"`
}

func respond(input request) map[string]any {
	return map[string]any{
		"request_id": input.RequestID,
		"status": "ok",
		"text": "",
		"error_text": nil,
		"extra": map[string]any{"result": map[string]any{"handled": true}},
	}
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	if !scanner.Scan() {
		return
	}
	var input request
	if err := json.Unmarshal(scanner.Bytes(), &input); err != nil {
		_ = json.NewEncoder(os.Stdout).Encode(map[string]any{
			"request_id": "invalid", "status": "error", "text": "", "error_text": err.Error(),
			"extra": map[string]any{"error_code": "request_invalid", "message_key": "skill.request_invalid"},
		})
		return
	}
	if err := json.NewEncoder(os.Stdout).Encode(respond(input)); err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
}
