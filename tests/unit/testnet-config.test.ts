import * as fs from 'fs'
import * as path from 'path'
import { TESTNET_CONTRACTS } from '../../sdk/src/config/testnet'

test('SDK Testnet addresses match the deployment record', () => {
  const record = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../deployments.json'), 'utf8')).testnet_final
  expect(TESTNET_CONTRACTS.verifier).toBe(record.verifier)
  expect(TESTNET_CONTRACTS.governance).toBe(record.governance)
  expect(TESTNET_CONTRACTS.token).toBe(record.token)
  expect(TESTNET_CONTRACTS.swap).toBe(record.swap)
  expect(TESTNET_CONTRACTS.compliance).toBe(record.compliance)
  expect(TESTNET_CONTRACTS.viewingKeys).toBe(record.viewing_keys)
})
