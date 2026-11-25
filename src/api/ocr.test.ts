import { describe, expect, it } from "vitest";

import { type PageResult, pageText } from "./ocr";

describe("pageText", () => {
  it("joins blocks with their end separators", () => {
    const page: PageResult = {
      width: 10,
      height: 10,
      elapsed_ms: 1,
      engine: "local",
      blocks: [
        {
          text: "Hello",
          box: [
            [0, 0],
            [1, 0],
            [1, 1],
            [0, 1],
          ],
          score: 1,
          end: " ",
        },
        {
          text: "world",
          box: [
            [2, 0],
            [3, 0],
            [3, 1],
            [2, 1],
          ],
          score: 1,
          end: "\n",
        },
      ],
    };
    expect(pageText(page)).toBe("Hello world\n");
  });
});
