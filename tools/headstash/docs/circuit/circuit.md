# Circuit Specs: Understanding How Headstash Circuits Work

Circuits are the invisible referees in headstash claims, ensuring everything happens fairly and securely. They turn your actions—like claiming tokens—into math problems that can be checked without revealing secrets. Think of them as smart locks: they only "unlock" if all the rules are followed perfectly.

This guide explains headstash circuits in simple terms, focusing on what makes them perform their core functions. We'll explore how they enforce rules, why they're secure, and what they check during claims.

## Quick Overview: What Circuits Do

Computers handle everything as 1s and 0s—bits. Circuits take your actions (like "I own this token") and break them into precise math rules using these bits. If the math checks out, the action is valid.

Headstash circuits focus on 3 main actions, each with rules to ensure correctness:

1. **Key-Pairing**: Verifying you own a secret key that matches a public key.
2. **Note-Merkle-Tree-Inclusion**: Confirming your note belongs in the system's token tree.
3. **Nullifier/Note-Commitment Integrity**: Preventing double-spending by linking claims uniquely.

These actions are constrained as math equations that must hold true. If any rule fails, the claim is rejected—like a puzzle piece that doesn't fit.

## Canonicity Gates: Ensuring Unique and Valid Representations

Canonicity gates are like quality checks for data. They ensure inputs to the circuit are represented correctly and uniquely, preventing confusion or cheating.

### Why It Matters

Imagine two different token amounts that look the same in bits—that would be a disaster for security. Canonicity gates make sure each value has a unique "fingerprint" and fits its expected size. For example, a 64-bit token amount must stay within 64 bits, not overflow or underflow.

In headstash, the Sinsemilla hash function processes data in 10-bit chunks. Canonicity gates check that values split correctly across these chunks, like ensuring a long word breaks into syllables properly.

### How It Works

- **Decomposition**: Break a value (e.g., a 254-bit randomness value) into smaller pieces that fit the 10-bit chunks.
- **Range Checks**: Verify each piece is the right size, like checking puzzle pieces aren't too big or small.
- **Reconstruction**: Ensure pieces add back to the original value, with math like `piece1 + 2^10 * piece2 + ...`.
- **Linking**: Connect pieces across chunks so the whole value flows seamlessly.

This creates a "unique representation" for hashing, so no two different inputs produce the same result. It's fundamental for secure commitments in claims.

## The 3 Core Actions in Detail

### 1. Key-Pairing: Proving Ownership Without Revealing Secrets

This action verifies that a secret key (esk) matches a public key (epk) on the curve. It's like proving you have the right key to a lock without showing the key.

- **How It Works**: The circuit checks elliptic curve math (e.g., does `esk * generator = epk`?). Inputs are decomposed into bits for constraints.
- **Why Secure**: You prove knowledge of esk without exposing it, thanks to canonicity ensuring unique representations.
- **In Claims**: Confirms you own the address eligible for tokens.

### 2. Note-Merkle-Tree-Inclusion: Confirming Your Place in the Token Tree

This ensures your note is part of the system's merkle tree, like verifying a leaf belongs to a big tree.

- **How It Works**: Hash your note with sibling paths to match the root. Constraints check each hash step fits the tree rules.
- **Why Secure**: Proves inclusion without revealing the tree's structure. Canonicity ensures hashes are unique.
- **In Claims**: Guarantees your allocation exists in the official headstash setup.

### 3. Nullifier/Note-Commitment Integrity: Preventing Double-Spending

This creates unique identifiers (nullifiers) for notes, ensuring each token is claimed only once, like serial numbers on bills.

- **How It Works**: Derive nullifiers from note data and secrets. Commitments hash note details privately. Constraints enforce uniqueness and integrity.
- **Why Secure**: Nullifiers are unlinkable and deterministic—spend once, and it's marked forever.
- **In Claims**: Stops reuse of the same allocation.

## Key Circuit Components

### Instances: Public Facts Everyone Agrees On

These are shared values, like the merkle tree root or your recipient address. They're public inputs that anchor the proof—everyone can see them, but they help verify your private claims.

### Witnesses: Private Secrets You Prove You Know

These are hidden values, like your secret key or randomness. The circuit proves you know them without revealing what they are, using constraints to "whisper" their validity.

### Verifying Key: The Stamp of Approval

This is a public key for checking proofs. It confirms a proof is valid for given instances, like a judge verifying a document's authenticity.

### Proving Key: Your Proof Generator

This private key generates proofs from witnesses and instances. It's like a writer's toolkit for creating evidence that passes verification.

## Constraint System: The Rules That Make Circuits Work

Constraints are the math equations that must be true for a proof to hold. They're like unbreakable rules: if all constraints pass, the action is valid.

- **How They Enforce Functions**: For each action (key-pairing, inclusion, integrity), constraints turn steps into equations. E.g., "Does this key pair match?" becomes a curve equation.
- **Security Guarantees**: Constraints ensure uniqueness and validity. Canonicity prevents bad representations; range checks keep values in bounds.
- **Limitations**: Circuits are efficient but require setup (keys). They can't handle infinite cases—everything must fit the math.

In headstash, circuits make claims private and fair. They prove ownership, inclusion, and integrity without spoilers, ensuring tokens go to the right people securely.
