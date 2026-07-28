import readline from "node:readline";

export function respond(request) {
  return {
    request_id: request.request_id,
    status: "ok",
    text: "",
    error_text: null,
    extra: { result: { handled: true } },
  };
}

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.once("line", (line) => {
  let response;
  try {
    response = respond(JSON.parse(line));
  } catch (error) {
    response = {
      request_id: "invalid",
      status: "error",
      text: "",
      error_text: String(error),
      extra: { error_code: "request_invalid", message_key: "skill.request_invalid" },
    };
  }
  process.stdout.write(`${JSON.stringify(response)}\n`);
  lines.close();
});
