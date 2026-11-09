import fs from 'fs';
import { readFile } from 'fs/promises';
import csv from 'csv-parser';
import { SAC_ENCODED_FILE } from './constants.js';

const processSacNFTdata = async (file) => {
    console.log(file)

    const stringCounts = {};

    // Parse the JSON data
    const data = await readFile(file, 'utf8'); // ✅ Properly await
    // Count the occurrences of each string
    JSON.parse(data).forEach((string) => {
        if (stringCounts[string]) {
            stringCounts[string]++;
        } else {
            stringCounts[string] = 1;
        }
    });

    // Sort the string counts by value in descending order
    const sortedStringCounts = Object.entries(stringCounts).sort((a, b) => b[1] - a[1]);

    // Create the output data string
    const outputData = "addr,amount\n" + sortedStringCounts.map(([string, count]) => `${string},${count}`).join('\n') + '\n';

    // Write the result to a new CSV file
    fs.writeFile(SAC_ENCODED_FILE, outputData, 'utf8', (err) => {
        if (err) {
            console.error(err);
        } else {
            console.log(`Output written to:  ${SAC_ENCODED_FILE}`);
        }
    });
    // base64-encodes solana addresses in format that will be used to verify offline signature

};

// base64 encode solana addresses
const encodeAddrs = async (inputFile, outputFile) => {
    const inputStream = fs.createReadStream(inputFile);
    const outputStream = fs.createWriteStream(outputFile);

    inputStream
        .pipe(csv({ mapHeaders: ({ header }) => header.trim() }))
        .on('data', (row) => {
            const addr = row.addr;
            const encodedAddr = Buffer.from(addr).toString('base64');
            outputStream.write(`${encodedAddr},${row.Amount},${row.Points},${row.Coins}\n`);
        })
        .on('end', () => {
            console.log('CSV file has been processed and written to output.csv');
        });
};


export { processSacNFTdata, encodeAddrs }