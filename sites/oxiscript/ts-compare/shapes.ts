type Shape =
  | { kind: "circle"; r: number }
  | { kind: "rect"; w: number; h: number };

export function area(s: Shape): number {
  switch (s.kind) {
    case "circle":
      return 3.14159 * s.r * s.r;
    case "rect":
      return s.w * s.h;
  }
}
