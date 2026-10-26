import { describe, expect, it } from "vitest";
import { initials, isBare, railItems } from "./nav";

const current = (path: string) =>
  railItems(path)
    .filter((i) => i.current)
    .map((i) => i.href);

describe("railItems", () => {
  it("lists the five destinations in order", () => {
    expect(railItems("/").map((i) => i.href)).toEqual([
      "/",
      "/events",
      "/players",
      "/history",
      "/profile",
    ]);
  });

  it("marks only Home on /", () => {
    expect(current("/")).toEqual(["/"]);
  });

  it("marks a section on a nested path", () => {
    expect(current("/events/abc")).toEqual(["/events"]);
  });

  it("marks a section on its exact path", () => {
    expect(current("/players")).toEqual(["/players"]);
  });

  it("marks nothing on an unrelated page", () => {
    expect(current("/account")).toEqual([]);
  });

  it("does not match a longer sibling name", () => {
    expect(current("/eventsfoo")).toEqual([]);
  });
});

describe("isBare", () => {
  it("is true on the fence, viewer and court screens", () => {
    for (const p of ["/s/x", "/v/x", "/c/x", "/cv/x"])
      expect(isBare(p)).toBe(true);
  });

  it("is false elsewhere", () => {
    for (const p of ["/events", "/", "/c", "/account"])
      expect(isBare(p)).toBe(false);
  });
});

describe("initials", () => {
  it("takes the first and last word", () => {
    expect(initials("Ana Maria Silva")).toBe("AS");
    expect(initials("coach")).toBe("C");
  });

  it("survives an empty name", () => {
    expect(initials("  ")).toBe("?");
  });
});
