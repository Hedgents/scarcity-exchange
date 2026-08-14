# Scarcity Exchange security status

The program is locally verified, but it is not approved for mainnet use. The frontend now rejects a mainnet operator or deployment manifest unless it declares a multisig with at least two approvals, a manual challenge window of at least 24 hours, and published HTTPS links for an independent audit, dispute policy, and incident response process.

The first initializer must be the deployed program's upgrade authority: `initialize_config` checks both the executable program account and its upgradeable-loader ProgramData account. The program also rejects a collateral mint whose decimals are not six. Market, order, vault, and token accounts use typed owners, canonical PDAs, stored config relationships, signer constraints, and snapshotted fee/resolver fields.

Those checks are deployment gates, not substitutes for onchain enforcement. Version one still gives the market's snapshotted resolver the ability to resolve after `resolve_after`; it does not implement an optimistic onchain challenge instruction. A dishonest authorized resolver can therefore publish an incorrect outcome unless the operational multisig and manual publication delay prevent it. If that resolver disappears, the protocol admin has a delayed recovery instruction after seven days; it can only mark the market invalid and enable fee-free one-for-one refunds, never select an outcome.

The pause flag deliberately stops new market creation, complete-set minting, new curve stake, and new order placement. Existing orders can still fill or cancel, curve stake can still be withdrawn before close, and users can still merge, resolve, claim, and redeem. Treat this as a risk-growth circuit breaker, not a universal freeze. Invalid binary markets pay one half of collateral per whole YES or NO base unit; odd base-unit dust rounds down and remains in the vault, which the frontend discloses.

Curve markets use separate `curve_market`, `curve_vault`, and `curve_position` PDA namespaces. A position is canonical for one market, owner, and bucket. The resolver cannot supply a bucket independently: the program derives it from the bounded normalized result. A full-support linear kernel prevents a zero payout denominator, and the market-wide stake cap keeps weighted arithmetic inside `u64` before proportional `u128` multiplication. The exact-bucket jackpot is capped at `exact stake × min(10, bucket count - 1)`, so dust spread across all buckets cannot extract a fixed fraction of the pool; unused target jackpot goes to the accuracy pool. Valid fees are transferred only at resolution; invalid and recovered markets charge no fee and refund stake one-for-one. Claims use snapshotted market economics, round down, and enforce the recorded payout-pool ceiling. Extra tokens donated to a vault do not affect liabilities.

Version one deliberately has no deadline-based curve dust sweep. That leaves rounding dust and abandoned claims in the vault rather than allowing an authority to confiscate an unclaimed user payout. Curve positions are nontransferable forecast-pool entries, not fixed-odds tokens.

The local validator suite covers unauthorized first initialization, unauthorized resolver rotation, premature resolution, canonical binary market creation, mint and merge, ask and bid fills, overfill rejection, partial fill and cancellation, winning and losing claim redemption, and invalid-market payouts. Curve coverage includes unauthorized creation, canonical owner/bucket positions, add and withdrawal, cross-owner mutation/claim rejection, close enforcement, pre-resolution claim rejection, resolver snapshots after global rotation, exact and near weighted payouts, no-exact fallback, protocol fees, premature/wrong-resolver/repeated invalidation rejection, invalid refunds, recovery authorization/delay rejection, a successful elapsed-market recovery transition, and double-claim rejection.

Before mainnet:

- commission and publish an independent Solana program review;
- place upgrade authority, protocol admin, and resolver behind reviewed multisig controls;
- use a reproducible program build and verify the deployed bytecode;
- build with the `0.31.1` Anchor binary pinned in `Anchor.toml` and preserve the resulting reproducible artifact manifest;
- review or remove the unmaintained transitive Solana/Anchor crates reported by `cargo audit` as framework upgrades become available;
- publish the dispute and incident procedures referenced by the manifest;
- decide whether to add an onchain challenge mechanism or permanently document the multisig/manual-delay trust model;
- run a devnet beta and one deliberately funded mainnet canary only after the above gates pass.

No seed phrase or private key belongs in this repository. User and operator mutations are wallet-signed.
