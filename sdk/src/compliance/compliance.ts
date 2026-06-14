import { ZKELLAWallet } from '../wallet/wallet'

export interface SanctionsList {
  root:            string
  version:         string
  publishedLedger: number
  fetchPath:       (address: string) => Promise<{ path: string[]; boundaryLeaves: string[] }>
}

export interface ComplianceProof {
  proof:          Uint8Array
  sanctionsRoot:  string
  version:        string
  tkCommitment:   string
  toJSON:         () => object
}

export class ZKELLACompliance {
  constructor(private config: { wallet: ZKELLAWallet }) {}

  static async fetchSanctionsList(url: string): Promise<SanctionsList> {
    const res  = await fetch(url)
    const data = await res.json()
    return {
      root:            data.root,
      version:         data.version,
      publishedLedger: data.published_ledger,
      fetchPath:       async (address: string) => {
        const r = await fetch(`${url}/path/${address}`)
        return r.json()
      },
    }
  }

  async generateNonSanctionedProof(_sanctions: SanctionsList): Promise<ComplianceProof> {
    // Generate Groth16 non-membership proof — M2
    return {
      proof:         new Uint8Array(192),
      sanctionsRoot: '',
      version:       '',
      tkCommitment:  '',
      toJSON:        () => ({}),
    }
  }

  async publishProof(_proof: ComplianceProof): Promise<{ submit: () => Promise<void> }> {
    // Submit to ViewingKeyRegistry contract — M2
    return { submit: async () => {} }
  }
}
