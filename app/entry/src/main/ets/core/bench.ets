// How many bridge rounds to run on start.
//
// Set from the ability, before any page exists, when the app is started with
// `aa start --pi bench N` — that is how platform/ohos/bench.sh asks for numbers
// without a finger on the Measure button. Zero means the normal case: a person
// presses it.
//
// A plain module variable, not observable state: nothing renders it, and nobody
// changes it after the want has been read. Making it observable would be a
// re-render for a number no component displays.

let wanted: number = 0;

export function setBenchRuns(n: number): void {
  wanted = n > 0 ? n : 0;
}

export function benchRuns(): number {
  return wanted;
}
