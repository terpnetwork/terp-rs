

// /// Database trait for headstash wallet storage
// ///
// /// This trait abstracts the storage backend. Implementations could use:
// /// - In-memory (for testing)
// /// - MetaMask snap_manageState (encrypted storage)
// /// - IndexedDB (browser)
// /// - File system (native)
// pub trait HeadstashApiDbInstance: Send + Sync {
//     // /// Store note data for a specific headstash
//     // fn gen_claim(&mut self, headstash_id: &String, note_data: NoteData) -> Result<(), Error>;

//     // /// Get note data by nullifier
//     // fn get_note_by_nullifier(
//     //     &self,
//     //     headstash_id: &String,
//     //     nullifier: &Nullifier,
//     // ) -> Result<Option<NoteData>, Error>;

//     /// List all unspent notes for a headstash
//     // fn list_unspent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error>;

//     /// List all spent notes for a headstash
//     // fn list_spent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error>;

//     /// Mark a note as spent
//     // fn mark_note_spent(
//     //     &mut self,
//     //     headstash_id: &String,
//     //     nullifier: &Nullifier,
//     // ) -> Result<(), Error>;

//     // /// Get all headstash IDs we have notes for
//     // fn list_headstash_ids(&self) -> Result<Vec<String>, Error>;
// }

// impl HeadstashApiDbInstance for MemoryHeadstashDb {
//     // fn gen_claim(&mut self, headstash_id: &String, note_data: NoteData) -> Result<(), Error> {
//     //     let notes = self
//     //         .storage
//     //         .entry(headstash_id.clone())
//     //         .or_insert_with(HashMap::new);
//     //     notes.insert(note_data.nullifier.to_bytes(), note_data);
//     //     Ok(())
//     // }

//     // fn get_note_by_nullifier(
//     //     &self,
//     //     headstash_id: &String,
//     //     nullifier: &Nullifier,
//     // ) -> Result<Option<NoteData>, Error> {
//     //     Ok(self
//     //         .storage
//     //         .get(headstash_id)
//     //         .and_then(|notes| notes.get(&nullifier.to_bytes()))
//     //         .cloned())
//     // }

//     // fn list_unspent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error> {
//     //     Ok(self
//     //         .storage
//     //         .get(headstash_id)
//     //         .map(|notes| notes.values().filter(|note| !note.spent).cloned().collect())
//     //         .unwrap_or_default())
//     // }

//     // fn list_spent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error> {
//     //     Ok(self
//     //         .storage
//     //         .get(headstash_id)
//     //         .map(|notes| notes.values().filter(|note| note.spent).cloned().collect())
//     //         .unwrap_or_default())
//     // }

//     // fn mark_note_spent(
//     //     &mut self,
//     //     headstash_id: &String,
//     //     nullifier: &Nullifier,
//     // ) -> Result<(), Error> {
//     //     if let Some(notes) = self.storage.get_mut(headstash_id) {
//     //         if let Some(note) = notes.get_mut(&nullifier.to_bytes()) {
//     //             note.spent = true;
//     //         }
//     //     }
//     //     Ok(())
//     // }

//     // fn list_headstash_ids(&self) -> Result<Vec<String>, Error> {
//     //     Ok(self.storage.keys().cloned().collect())
//     // }
// }

    /// Store a note in the wallet database
    ///
    /// # Arguments
    ///
    /// * `headstash_id` - Contract address of the headstash
    /// * `esk_hex` - Secret key in hex format (from MetaMask)
    /// * `rho_hex` - Randomness value in hex
    /// * `fdi` - Fixed denomination index (leaf position in tree)
    /// * `recp_hex` - Recipient address in hex
    /// * `value_amount` - Note value amount
    /// * `value_denom` - Denomination (e.g., "uterp")
    /// * `rseed_hex` - Random seed in hex
    ///
    /// # Examples
    ///
    /// ```javascript
    /// await wallet.gen_claim(
    ///     "terp1contract123",
    ///     esk_hex,
    ///     rho_hex,
    ///     0,
    ///     recp_hex,
    ///     1000,
    ///     "uterp",
    ///     rseed_hex
    /// );
    /// ```
    // pub async fn gen_claim(
    //     &self,
    //     headstash_id: String,
    //     esk_hex: String,
    //     rho_hex: String,
    //     fdi: u64,
    //     recp_hex: String,
    //     value_amount: u64,
    //     value_denom: String,
    //     rseed_hex: String,
    // ) -> Result<(), Error> {
    //     let esk = EligibleSk::from_hex(&esk_hex);
    //     let rho_bytes = hex::decode(&rho_hex)
    //         .map_err(|e| Error::KeyDecoding(format!("Invalid rho hex: {}", e)))?;
    //     let rho = Rho::from_bytes(
    //         rho_bytes
    //             .as_slice()
    //             .try_into()
    //             .map_err(|_| Error::KeyDecoding("Invalid rho length".into()))?,
    //     )
    //     .expect("rho from bytes");

    //     let recp_bytes = hex::decode(&recp_hex)
    //         .map_err(|e| Error::KeyDecoding(format!("Invalid recp hex: {}", e)))?;

    //     let hv = HeadstashValue::from_raw(value_amount, &value_denom)
    //         .map_err(|e| Error::KeyDecoding(format!("Invalid value: {:?}", e)))?;

    //     let rseed_bytes: [u8; 32] = hex::decode(&rseed_hex)
    //         .map_err(|e| Error::KeyDecoding(format!("Invalid rseed hex: {}", e)))?
    //         .as_slice()
    //         .try_into()
    //         .map_err(|_| Error::KeyDecoding("Invalid rseed length".into()))?;

    //     self.inner
    //         .gen_claim(headstash_id, esk, rho, fdi, &recp_bytes, hv, rseed_bytes)
    //         .await
    // }
