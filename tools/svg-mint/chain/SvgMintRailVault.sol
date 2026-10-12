// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice XOR escrow for SVG mint ETH rail. RELEASE or REFUND, not both.
/// Refund/release authorized by ed25519-equivalent: we verify **secp256k1**
/// federation operator signatures over the escrow id (CosmWasm oracles are ed25519;
/// Anvil operators use this vault's ecrecover set). No SP1 / Groth16.
contract SvgMintRailVault {
    enum Status {
        None,
        Locked,
        Released,
        Refunded
    }

    struct Escrow {
        address depositor;
        uint256 amount;
        uint64 deadline;
        Status status;
        bytes32 nDeposit;
        string terpRecipient;
    }

    address[] public operators;
    uint256 public threshold;
    mapping(bytes32 => Escrow) public escrows;
    mapping(bytes32 => bool) public usedNullifiers;

    event Locked(
        bytes32 indexed id,
        address depositor,
        uint256 amount,
        bytes32 nDeposit,
        string terpRecipient
    );
    event Refunded(bytes32 indexed id, address to, uint256 amount);
    event Released(bytes32 indexed id, address to, uint256 amount);

    constructor(address[] memory ops, uint256 thresh) {
        require(thresh > 0 && thresh <= ops.length, "threshold");
        operators = ops;
        threshold = thresh;
    }

    /// Third arg is the payment memo: hex of Pedersen C (`d1`||32 bytes),
    /// never a `terp1` bech32. DREGG matches C off-chain to the mint dest.
    function lock(bytes32 nDeposit, uint64 deadline, string calldata terpRecipient)
        external
        payable
        returns (bytes32 id)
    {
        require(msg.value > 0, "value");
        require(!usedNullifiers[nDeposit], "spent");
        require(bytes(terpRecipient).length >= 20, "terp");
        id = keccak256(abi.encodePacked(msg.sender, nDeposit, block.number, terpRecipient));
        Escrow storage e = escrows[id];
        require(e.status == Status.None, "exists");
        e.depositor = msg.sender;
        e.amount = msg.value;
        e.deadline = deadline;
        e.status = Status.Locked;
        e.nDeposit = nDeposit;
        e.terpRecipient = terpRecipient;
        emit Locked(id, msg.sender, msg.value, nDeposit, terpRecipient);
    }

    function refundTimeout(bytes32 id) external {
        Escrow storage e = escrows[id];
        require(e.status == Status.Locked, "not locked");
        require(block.timestamp > e.deadline, "deadline");
        require(msg.sender == e.depositor, "depositor");
        _payRefund(id, e);
    }

    /// Sold-out / settle refund: M-of-N secp256k1 over keccak256("svg-mint-eth-refund-v1" || id || nDeposit).
    function refundSoldOut(bytes32 id, bytes[] calldata sigs) external {
        Escrow storage e = escrows[id];
        require(e.status == Status.Locked, "not locked");
        bytes32 digest = keccak256(abi.encodePacked("svg-mint-eth-refund-v1", id, e.nDeposit));
        require(_quorum(digest, sigs), "qc");
        _payRefund(id, e);
    }

    function release(bytes32 id, address to, bytes[] calldata sigs) external {
        Escrow storage e = escrows[id];
        require(e.status == Status.Locked, "not locked");
        bytes32 digest = keccak256(abi.encodePacked("svg-mint-eth-release-v1", id, e.nDeposit, to));
        require(_quorum(digest, sigs), "qc");
        e.status = Status.Released;
        usedNullifiers[e.nDeposit] = true;
        uint256 amt = e.amount;
        (bool ok, ) = to.call{value: amt}("");
        require(ok, "pay");
        emit Released(id, to, amt);
    }

    function _payRefund(bytes32 id, Escrow storage e) internal {
        e.status = Status.Refunded;
        usedNullifiers[e.nDeposit] = true;
        uint256 amt = e.amount;
        address to = e.depositor;
        (bool ok, ) = to.call{value: amt}("");
        require(ok, "pay");
        emit Refunded(id, to, amt);
    }

    function _quorum(bytes32 digest, bytes[] calldata sigs) internal view returns (bool) {
        bytes32 eth = keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", digest));
        uint256 got;
        address[] memory seen = new address[](sigs.length);
        for (uint256 i = 0; i < sigs.length; i++) {
            address rec = _recover(eth, sigs[i]);
            bool allowed;
            for (uint256 j = 0; j < operators.length; j++) {
                if (operators[j] == rec) allowed = true;
            }
            if (!allowed) continue;
            bool dup;
            for (uint256 k = 0; k < got; k++) {
                if (seen[k] == rec) dup = true;
            }
            if (dup) continue;
            seen[got] = rec;
            got++;
        }
        return got >= threshold;
    }

    function _recover(bytes32 hash, bytes memory sig) internal pure returns (address) {
        require(sig.length == 65, "sig");
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }
        if (v < 27) v += 27;
        return ecrecover(hash, v, r, s);
    }
}
