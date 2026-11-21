export type ApiResponseFormatErrorKind = "html_response" | "invalid_json";

export class ApiResponseFormatError extends Error {
  constructor(readonly kind: ApiResponseFormatErrorKind) {
    super(kind);
    this.name = "ApiResponseFormatError";
  }
}

export async function readJsonApiResponse<T>(response: Response): Promise<T> {
  const raw = await response.text();
  try {
    return JSON.parse(raw) as T;
  } catch {
    const contentType = response.headers.get("content-type")?.toLowerCase() ?? "";
    const looksLikeHtml = contentType.includes("text/html") || /^\s*<(?:!doctype|html)\b/i.test(raw);
    throw new ApiResponseFormatError(looksLikeHtml ? "html_response" : "invalid_json");
  }
}

/** A received HTTP rejection is a known outcome; receipt recovery is for transport ambiguity. */
export class ApiHttpError extends Error {
  constructor(readonly status: number, readonly errorCode: string) {
    super(errorCode);
    this.name = "ApiHttpError";
  }
}

export async function readTaskSubmissionResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let code = `task_submit_http_${response.status}`;
    if (response.headers.get("content-type")?.toLowerCase().includes("application/json")) {
      try {
        const body: unknown = JSON.parse(await response.text());
        if (body && typeof body === "object" && "error" in body && typeof body.error === "string") code = body.error;
      } catch { /* Keep the status when a proxy supplies a malformed body. */ }
    }
    throw new ApiHttpError(response.status, code);
  }
  return readJsonApiResponse<T>(response);
}
