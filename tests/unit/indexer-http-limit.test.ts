// Regression test for a Low/Medium-severity audit finding: `/notes`'s
// `?limit=` query param flowed straight into a SQL `LIMIT ?` with no
// validation. Beyond simple resource exhaustion for a huge value, SQLite
// specifically treats a *negative* `LIMIT` as "unlimited" — so an
// unvalidated negative value wasn't just wasteful, it was a real
// unbounded-query vector.

import { parseNotesLimit } from '../../indexer/src/http.ts'

describe('parseNotesLimit', () => {

  test('defaults to 500 when no limit is given', () => {
    expect(parseNotesLimit(null)).toBe(500)
  })

  test('passes through a normal, in-range value', () => {
    expect(parseNotesLimit('200')).toBe(200)
  })

  test('clamps a value above the maximum down to 1000', () => {
    expect(parseNotesLimit('999999999')).toBe(1000)
  })

  test('clamps a negative value up to 1 (SQLite treats negative LIMIT as unlimited)', () => {
    expect(parseNotesLimit('-1')).toBe(1)
    expect(parseNotesLimit('-999999')).toBe(1)
  })

  test('clamps zero up to 1', () => {
    expect(parseNotesLimit('0')).toBe(1)
  })

  test('falls back to the default for non-numeric input', () => {
    expect(parseNotesLimit('not-a-number')).toBe(500)
  })

  test('an empty string parses as 0 (JS `Number("")`), clamped up to 1', () => {
    expect(parseNotesLimit('')).toBe(1)
  })

  test('truncates a fractional value', () => {
    expect(parseNotesLimit('12.9')).toBe(12)
  })

})
