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
