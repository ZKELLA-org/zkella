import { xdr, nativeToScVal, scValToNative } from '@stellar/stellar-sdk'
import { ZKELLAWallet } from '../wallet/wallet'
import { generateNonMembershipProof, buildSanctionsTree } from '../prover/compliance'
import { structScVal } from '../wallet/wallet'
import { bigIntToBuffer } from '../crypto/poseidon'

export interface ComplianceProof {
  proof:         Uint8Array
  sanctionsRoot: Uint8Array
  tkCommitment:  Uint8Array
}

export interface ComplianceConfig {
  wallet:              ZKELLAWallet
  complianceAddress:   string
  /** The sanctioned addresses (247-bit field values) the maintainer publishes. */
  sanctionedAddresses: bigint[]
  wasmPath:            string
  zkeyPath:            string
}

/**
 * Generates a non-membership proof against the maintainer's sanctions list and
 * publishes it to the compliance contract. The contract accepts only proofs
 * against the root the admin set, so `publishProof` fails unless the list here
 * matches the one published on-chain.
 */
export class ZKELLACompliance {
  constructor(private config: ComplianceConfig) {}

  /** The sanctions-tree root this wrapper's list produces; the admin must publish it. */
  async sanctionsRoot(): Promise<Uint8Array> {
    const tree = await buildSanctionsTree(this.config.sanctionedAddresses)
    return bigIntToBuffer(tree.root)
  }

  async generateNonSanctionedProof(): Promise<ComplianceProof> {
    const result = await generateNonMembershipProof({
      sk:         this.config.wallet.complianceSecret(),
      sanctioned: this.config.sanctionedAddresses,
      wasmPath:   this.config.wasmPath,
      zkeyPath:   this.config.zkeyPath,
    })
    return { proof: result.proof, sanctionsRoot: result.sanctionsRoot, tkCommitment: result.tkCommitment }
  }

  /** Submits the proof as the wallet's own account; the contract checks the owner's auth. */
  async publishProof(p: ComplianceProof): Promise<{ submit: () => Promise<void> }> {
    const submit = async (): Promise<void> => {
      await this.config.wallet.submitContractCall(this.config.complianceAddress, 'publish_compliance_proof', [
        nativeToScVal(this.config.wallet.accountAddress(), { type: 'address' }),
        nativeToScVal(p.proof, { type: 'bytes' }),
        structScVal(
          { sanctions_root: p.sanctionsRoot, tk_commitment: p.tkCommitment },
          { sanctions_root: 'bytes', tk_commitment: 'bytes' },
        ),
      ])
    }
    return { submit }
  }
}
