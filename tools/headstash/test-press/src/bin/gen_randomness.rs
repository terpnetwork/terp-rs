fn main() {
    let data = headstash_randomness::ultra_secure_random();
    println!("Randomness: {:#?}", hex::encode(data));
}
