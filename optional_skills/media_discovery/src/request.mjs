import createJsonParser from "json-bigint";

// Runtime actor IDs are i64 JSON integers. Preserve their exact decimal digits
// on Node 20 as well as newer engines; never use a rounded Number as ownership.
const parser = createJsonParser({ strict: true, protoAction: "error", constructorAction: "error" });
export function parseSkillRequest(line) {
  return parser.parse(line, (_key, value) => {
    // Only parser-created BigNumber instances have these methods; ordinary JSON
    // objects cannot supply functions. Leave JSON strings untouched.
    if (value && typeof value === "object" && typeof value.toFixed === "function" && typeof value.isInteger === "function") {
      const number = value.toNumber();
      return value.isInteger() && !Number.isSafeInteger(number) ? value.toFixed() : number;
    }
    return value;
  });
}
