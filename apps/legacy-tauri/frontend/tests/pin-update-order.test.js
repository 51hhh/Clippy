import { describe, expect, it } from "vitest";
import { shouldApplyPinUpdateResponse } from "../react/pin/update-order";

describe("pin update response ordering", () => {
  it("accepts the latest response when no local update is queued", () => {
    expect(shouldApplyPinUpdateResponse(4, 4, {})).toBe(true);
  });

  it("rejects stale responses and responses racing a queued update", () => {
    expect(shouldApplyPinUpdateResponse(3, 4, {})).toBe(false);
    expect(shouldApplyPinUpdateResponse(4, 4, { scale: 1.2 })).toBe(false);
  });
});
