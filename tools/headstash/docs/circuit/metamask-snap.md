# Snap-n-Pull: MetaMask Snap Plugin for Headstash Interaction

**Version:** 2.1
**Status:** Production Implementation
**Last Updated:** 2025-01-20

## Overview

The snap-n-pull MetaMask Snap plugin helps you claim Headstash airdrop tokens safely without ever sharing your private keys or linking your claims on the blockchain. It's like claiming tokens normally, but with complete privacy—your secrets stay locked in MetaMask.

**Core Principle:** "A good UX does not require the user to learn anything they do not already know."

### Key Capabilities

The snap lets you:

- Create secure proofs for your claims from private keys without revealing them
- Generate commitments to your notes
- Derive public keys for verification
- Keep your private keys completely secure—they're never stored, only used temporarily
- Integrate seamlessly with MetaMask's secure environment
- Use powerful cryptographic tools compiled for the web
- Check on-chain data to prevent double-spending
- Sync your wallet across devices through secure helpers
- Route proofs efficiently for faster, cheaper submissions

## Connections to Services

The snap works with two key services to keep your experience safe and smooth:

### 1. Blockchain Connection

This links to the Cosmos/Terp Network blockchain to check your claims in real-time. It verifies if your tokens are still available and stops you from claiming twice by checking what's already been used.

- It pulls the latest blockchain info to confirm everything is valid.
- This connection is secure and only gets the necessary public information.

### 2. AVS Connection (Autonomous Verification Service)

This connects to trusted helpers for syncing your wallet and optimizing your claims.

- Wallet Syncing: Keeps your spent tokens and balances up to date across your devices, so you always have the full picture without sharing any secrets.
- Proof Routing/Aggregation: Sends your proofs through smart paths or groups them for lower costs and faster processing.
- All data is encrypted, and syncing only shares safe public details.

## How It Works

Here's the simple process when you claim tokens:

1. You select a headstash to claim from the app and provide basic details.
2. The app calls the snap to start the process.
3. The snap checks the blockchain and syncs your wallet state.
4. It securely gets your private key from MetaMask (just for this use).
5. The snap creates the needed proofs and identifiers.
6. Your key is immediately cleared from memory.
7. The app gets back safe public information to create your final proof.
8. You broadcast the claim to the blockchain.

## Core Parts

### MetaMask Snap Package

This is the main plugin that runs inside MetaMask. It handles requests from apps, validates them, and processes your claims securely.

### Cryptographic Core

The secure engine that handles all the math for creating proofs. It's built in Rust and runs safely in your browser.

## Basic Security

### What We Never Do

- Store your private keys
- Share or log your private keys
- Send your private keys over the internet
- Save your keys to disk
- Reveal your keys in any records

### What We Do

- Ask for your key only when needed for a claim
- Use the key temporarily for calculations
- Clear the key immediately after
- Return only safe public information
- Ask for your confirmation before every action
- Create public keys for verification only

### Privacy Features

Your claims stay private:

- No one can link your claims to your wallet address
- Each claim gets a unique identifier that's impossible to reuse
- The blockchain only sees public proof details, not your personal info
- You can claim multiple times without anyone knowing they're from the same person

## How It Fits Together

The MetaMask Snap is part of a complete system for handling headstash claims. It connects to the blockchain and helpers for real-time, secure data.

### Your Claiming Journey

1. Install the MetaMask Snap
2. Check what you're eligible for
3. Choose where to receive your tokens
4. Generate your proof through the snap and system
5. Submit your claim to the blockchain
6. Export your records for backup
7. Sync across your devices

## References

**Documentation:**

- [MetaMask Snaps API](https://docs.metamask.io/snaps/)
- [Headstash Spec](./spec.md)
- [Frontend Guide](./frontend.md)

---

**Version History:**

- **v2.1** (2025-01-20) - Added connections for blockchain checks and wallet syncing; focused on user experience
- **v2.0** (2025-01-20) - Complete update for Headstash with full implementation
- **v1.0** (2024) - Initial version (no longer used)