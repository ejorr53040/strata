import { helper, Counter } from './util.js';
import * as util from './util.js';
import confetti from 'canvas-confetti';

export function main() {
  const c = new Counter();
  helper(1);
  util.double(2);
  confetti();
  console.log('hi');
  missing();
}

class Panel extends Counter {
  open() {
    this.bump();
  }
}
