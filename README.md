# Hedgents Scarcity Exchange

This workspace contains the Solana settlement layer for Hedgents scarcity markets. It is deliberately separate from the metal execution terminal and the stablecoin rail.

The first protocol version supports two separate, fully collateralized primitives:

- transferable YES/NO claims with a secondary limit orderbook; and
- nontransferable scalar forecast positions whose payouts follow a deterministic accuracy curve.

For binary markets:

1. An administrator creates a market by committing the hashes of its immutable question and resolution rules.
2. Depositing one base unit of USDC mints one YES and one NO outcome unit.
3. A complete YES/NO pair can always be burned to recover its USDC before resolution.
4. Each market snapshots its resolver when it is created. That resolver commits a content-addressed resolution report hash; later global resolver rotation cannot change existing markets.
5. If a market is invalid, each outcome unit redeems for half a USDC unit, rounded down only at the smallest token unit.

This creates the core solvency invariant:

```text
unresolved vault collateral = complete sets outstanding
resolved vault liability     = winning claims not yet redeemed
```

There is no bridge code here, no discretionary custody, no unbacked outcome issuance, and no mainnet deployment yet.

For curve markets, an administrator commits an immutable metric and rule set, an odd number of buckets from 3 through 41, and an exact-bucket target jackpot from 0% through 50%. The buckets span the normalized interval `[-1, 1]`. A wallet may keep one canonical position per bucket, add stake, or withdraw stake until trading closes. The snapshotted resolver later submits the normalized integer result and a content-addressed resolution report.

Valid resolution distributes the deposited pool as follows:

```text
protocol fee = floor(total stake × snapshotted fee bps / 10,000)
target       = floor(post-fee pool × configured jackpot bps / 10,000)
leverage cap = exact-bucket stake × min(10, bucket count - 1)
jackpot      = min(target, leverage cap)
curve weight = bucket count - distance from the resolved bucket
payout       = pro-rata exact jackpot + pro-rata weighted curve pool
```

The leverage cap prevents tiny positions spread across every bucket from capturing a fixed share of other traders' collateral. Any unused target jackpot returns to the accuracy pool. The full-support weight is always positive, so every funded market has a valid payout denominator. Per-position division rounds down, leaving at most small base-unit dust in the vault; there is deliberately no deadline-based sweep that could confiscate an unclaimed position. Invalid curve markets charge no fee and refund each position one-for-one.

If the snapshotted resolver becomes unavailable, the protocol admin may invalidate only after seven days beyond `resolve_after`. This recovery path cannot choose an outcome or charge a fee; it only unlocks one-for-one refunds. Production still requires the admin and resolver authorities to be durable reviewed multisigs.

Curve positions are forecast-pool entries in version one. They are not tokens, are not transferable, and do not yet have resale liquidity. The UI must not describe them as fixed odds or a secondary orderbook.

The program also contains a protocol-owned secondary limit orderbook:

- An ask escrows YES or NO outcome tokens.
- A bid escrows the maximum USDC quote amount.
- Anyone can fill all or part of an unexpired order.
- Fees are calculated cumulatively in integer base units so partial-fill rounding cannot overcharge.
- The order snapshots its fee recipient and fee rate.
- The maker can cancel and reclaim remaining escrow. Fully filled and expired order accounts remain reclaimable by the maker so rent cannot become trapped permanently.

The backend never signs for users, makers, takers, the administrator, or the resolver. All program mutations are built as wallet-signed transactions.

## Local checks

```bash
cargo test
cargo check
bash scripts/anchor-build.sh
cd ../frontend
npm run sync:scarcity-idl
npm run test:scarcity-localnet
```

`scripts/anchor-build.sh` refuses to build unless the selected CLI is exactly Anchor `0.31.1`, matching `anchor-lang` and `anchor-spl`. Set `HEDGENTS_ANCHOR_CLI` to an exact binary path when the shell's default `anchor` points at a different AVM version.

The local-validator test creates disposable unfunded identities and a mock six-decimal collateral mint. It first creates a canonical Gold 15 market from the Metal Pulse recurring factory, then exercises binary issuance, merge, bid and ask escrow, full and partial fills, exact fees, cancellation, resolver rotation, resolution, and redemption. It also exercises curve creation authorization, canonical positions, add/withdraw, cross-owner custody rejection, close enforcement, pre-resolution claim rejection, resolver snapshots, exact and near payouts, the no-exact fallback, protocol fees, invalid refunds, repeated-transition rejection, and double-claim rejection. It verifies recovery authorization and delay rejection, then successfully recovers an elapsed empty market as invalid. It does not contact devnet or spend real SOL.

The program ID reserved for local/devnet builds is `CJHWP9ed1BzWVQhUeJPQ9jJb4YcVWiFNpQcG7mPEGk86`. It has not been deployed to devnet or mainnet.

## Public-launch blockers

- Independent Solana security review and reproducible deployment build.
- Multisig design for program upgrade authority, protocol admin, and resolver operations.
- An optimistic dispute/challenge window or another reviewed dispute design. Version one currently resolves through the market's snapshotted resolver.
- Mainnet USDC deployment manifest, RPC capacity, monitoring, incident response, and legal review.
- A decision on program immutability after the beta has proven the account and instruction design.
- Curve positions are not transferable and have no secondary orderbook in version one.
