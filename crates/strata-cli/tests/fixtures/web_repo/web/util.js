export function helper(x) {
  return x + 1;
}

export class Counter {
  constructor() {
    this.n = 0;
  }

  bump() {
    this.n = helper(this.n);
  }
}

export const double = (x) => x * 2;
