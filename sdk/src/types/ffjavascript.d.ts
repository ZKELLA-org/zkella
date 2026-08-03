declare module 'ffjavascript' {
  interface G1Point {}
  interface FieldElement {}

  interface Bn128G1 {
    g: G1Point
    timesScalar(p: G1Point, scalar: bigint): G1Point
    timesFr(p: G1Point, scalar: unknown): G1Point
    add(a: G1Point, b: G1Point): G1Point
    eq(a: G1Point, b: G1Point): boolean
    toRprCompressed(buf: Uint8Array, offset: number, p: G1Point): Uint8Array
    fromRprCompressed(buf: Uint8Array, offset: number): G1Point
    toObject(p: G1Point): bigint[]
    fromObject(o: bigint[]): G1Point
    isValid(p: G1Point): boolean
    isZero(p: G1Point): boolean
  }

  interface Bn128Fr {
    p: bigint
    e(v: bigint | number): unknown
  }

  interface Bn128F1 {
    p: bigint
    e(v: bigint | number): FieldElement
    add(a: FieldElement, b: FieldElement): FieldElement
    mul(a: FieldElement, b: FieldElement): FieldElement
    isSquare(a: FieldElement): boolean
    sqrt(a: FieldElement): FieldElement
    toObject(a: FieldElement): bigint
  }

  interface Bn128Curve {
    G1: Bn128G1
    Fr: Bn128Fr
    F1: Bn128F1
  }

  export function buildBn128(): Promise<Bn128Curve>
}
