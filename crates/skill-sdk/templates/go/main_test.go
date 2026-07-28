package main

import "testing"

func TestResponseEchoesRequestID(t *testing.T) {
	result := respond(request{RequestID: "test-1"})
	if result["request_id"] != "test-1" {
		t.Fatal("request id mismatch")
	}
}
