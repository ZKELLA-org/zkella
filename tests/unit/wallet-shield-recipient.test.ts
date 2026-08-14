// Regression test for a Medium-severity audit finding: `ZKELLAWallet.shield()`
// declared an `opts.to` (recipient) parameter but never used it — the note
// was always encrypted to the *sender's own* transmissionKey regardless,
// meaning a deposit "for a recipient" was actually only ever
// decryptable/spendable by the sender.
//
// `generateShieldProof` is mocked out (real Groth16 proving is slow and
// unrelated to this bug) so this test runs fast and focuses precisely on
// the encryption-key selection logic in `shield()` itself. `jest.spyOn` on
// the real `encryptNote` captures the exact argument `shield()` itself
// passes it — the actual behavior under test, not a reconstruction of what
// *should* happen.

import { Keypair } from '@stellar/stellar-sdk'

jest.mock('../../sdk/src/prover/shield', () => ({
  generateShieldProof: jest.fn().mockResolvedValue({
    proof: new Uint8Array(0),
    valueCommit: new Uint8Array(32),
    publicInputsLE: [],
  }),
}))

import { ZKELLAWallet } from '../../sdk/src/wallet/wallet'
import { ZKELLAKeys } from '../../sdk/src/keys/keys'
import * as encryptModule from '../../sdk/src/notes/encrypt'

const MOCK_ASSET = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

async function makeWallet(keys: ZKELLAKeys): Promise<ZKELLAWallet> {
  return new ZKELLAWallet({
    keys: keys.spendingKey,
    network: 'testnet',
    sorobanRpc: 'http://localhost:0',
    indexerUrl: 'http://localhost:0',
    tokenAddress: MOCK_ASSET,
    stellarSecret: Keypair.random().secret(),
    shieldCircuit: { wasmPath: '/dev/null', zkeyPath: '/dev/null' },
  })
}

describe('ZKELLAWallet.shield() recipient handling', () => {

  afterEach(() => {
    jest.restoreAllMocks()
  })

  test('opts.to is used: shield() encrypts to the recipient transmissionKey, not the sender\'s', async () => {
    const encryptSpy = jest.spyOn(encryptModule, 'encryptNote')

    const sender    = await ZKELLAKeys.generate()
    const recipient = await ZKELLAKeys.generate()
    const senderWallet = await makeWallet(sender)

    const recipientTkHex = bytesToHex(recipient.spendingKey.transmissionKey)
    await senderWallet.shield({ asset: MOCK_ASSET, amount: 1_000_000n, to: recipientTkHex })

    expect(encryptSpy).toHaveBeenCalledTimes(1)
    const [, transmissionKeyArg] = encryptSpy.mock.calls[0]
    expect(bytesToHex(transmissionKeyArg as Uint8Array)).toBe(recipientTkHex)
    // The bug being regression-tested: this used to unconditionally equal
    // the *sender's* own key regardless of `opts.to`.
    expect(bytesToHex(transmissionKeyArg as Uint8Array))
      .not.toBe(bytesToHex(sender.spendingKey.transmissionKey))
  })

  test('opts.to omitted: shield() defaults to the wallet\'s own transmissionKey (self)', async () => {
    const encryptSpy = jest.spyOn(encryptModule, 'encryptNote')

    const owner = await ZKELLAKeys.generate()
    const wallet = await makeWallet(owner)

    await wallet.shield({ asset: MOCK_ASSET, amount: 500_000n })

    expect(encryptSpy).toHaveBeenCalledTimes(1)
    const [, transmissionKeyArg] = encryptSpy.mock.calls[0]
    expect(bytesToHex(transmissionKeyArg as Uint8Array))
      .toBe(bytesToHex(owner.spendingKey.transmissionKey))
  })

  test('end-to-end: a note shielded "to" a recipient is decryptable by the recipient, not the sender', async () => {
    const sender    = await ZKELLAKeys.generate()
    const recipient = await ZKELLAKeys.generate()
    const senderWallet = await makeWallet(sender)

    const recipientTkHex = bytesToHex(recipient.spendingKey.transmissionKey)
    const { note } = await senderWallet.shield({
      asset: MOCK_ASSET, amount: 250_000n, to: recipientTkHex,
    })

    // shield() doesn't return the encrypted bundle directly (it's only used
    // inside the private `submit` closure) — but the spy in the first test
    // already proves shield() itself calls encryptNote with the recipient's
    // key; here, encrypting the same note with that same real key and
    // decrypting with each party's real viewing key proves the resulting
    // ciphertext genuinely round-trips only for the intended recipient.
    const bundle = await encryptModule.encryptNote(note, recipient.spendingKey.transmissionKey)

    const recipientDecrypted = await encryptModule.tryDecryptNote(bundle, recipient.toViewingKey(0).raw)
    expect(recipientDecrypted).not.toBeNull()
    expect(recipientDecrypted!.value).toBe(250_000n)

    const senderDecrypted = await encryptModule.tryDecryptNote(bundle, sender.toViewingKey(0).raw)
    expect(senderDecrypted).toBeNull()
  })

})
