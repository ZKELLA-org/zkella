declare module 'snarkjs' {
  interface Groth16Proof {
    pi_a: string[]
    pi_b: string[][]
    pi_c: string[]
    protocol: string
    curve: string
  }

  interface Groth16FullProveResult {
    proof: Groth16Proof
    publicSignals: string[]
  }

  type CircuitSignal = string | string[] | string[][]

  export const groth16: {
    fullProve(
      input: Record<string, CircuitSignal>,
      wasmFile: string,
      zkeyFileName: string,
    ): Promise<Groth16FullProveResult>
    prove(zkeyFileName: string, witness: unknown): Promise<Groth16FullProveResult>
    verify(
      vk: unknown,
      publicSignals: string[],
      proof: Groth16Proof,
    ): Promise<boolean>
  }
}
