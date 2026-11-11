
import childProcess from 'child_process';
import path from 'path';
import fs from 'fs';
import { spawn } from 'child_process';
import moment from 'moment';
import csv from 'csv-parser';


// 1. download daodao repo 
async function cloneRepository(url, path) {
    try {
        await new Promise((resolve, reject) => {
            childProcess.exec(`git clone ${url} ${path}`, (error, stdout, stderr) => {
                if (error) {
                    reject(error);
                } else {
                    resolve();
                }
            });
        });
        console.log(`Repository cloned successfully to ${path}`);
    } catch (error) {
        console.error(`Error cloning repository: ${error}`);
    }
}

//  define the directories to run the script on
const dirs = [
    './dao-contracts/contracts',
    './dao-contracts/packages',
    './dao-contracts/scripts',
];

// 2. generate csv file for each directory included in count 
function runBashScript(dir) {
    const date = moment().format('MM-DD-YY');
    const dirName = dir.replace('./', '').replace(/\//g, '_');
    const script = `find ${dir} -type f -exec awk -v OFS=, -v IGNORECASE=1 '{ 
      while ($0 ~ /DAO/) {
        match($0, /DAO/);
        print FILENAME, NR, substr($0, RSTART, RLENGTH);
        $0 = substr($0, RSTART + RLENGTH)
      }
    }' {} \\; > ./tallies/${date}/raw/${dirName}_results.csv`;

    // Make sure the directory exists before running the script
    const directory = `./tallies/${date}/raw`;
    if (!fs.existsSync(directory)) {
        fs.mkdirSync(directory, { recursive: true });
    }

    // Run the script using the Unix shell
    const shell = spawn('/bin/sh', ['-c', script], { stdio: 'inherit' });
    shell.on('close', (code) => {
        if (code !== 0) {
            console.error(`Process exited with code ${code}`);
        } else {
            console.log(`The script has completed and ${dir.replace(/\//g, '_')}_results.csv has been generated.`);
        }
    });
}

// 3. for each line in the csvs that has the same file location, count the # of times it exists.
// print these results for each file in a new csv file 
function countFileLocations() {
    const date = moment().format('MM-DD-YY');
    const directory = `./tallies/${date}/raw`;

    // Define the file names
    const fileNames = [
        'dao-contracts_contracts_results.csv',
        'dao-contracts_packages_results.csv',
        'dao-contracts_scripts_results.csv',
    ];

    let fileIndex = 0;
    function processFile() {
        if (fileIndex >= fileNames.length) {
            return;
        }

        const fileName = fileNames[fileIndex];
        const filePath = `${directory}/${fileName}`;
        const fileCounts = {};

        // Create a new CSV file to write the results in the "count" folder
        const parentDirectory = path.dirname(directory);
        const countDirectory = `${parentDirectory}/count`;
        if (!fs.existsSync(countDirectory)) {
            fs.mkdirSync(countDirectory);
        }
        const resultsCsv = `${countDirectory}/count_${fileName}`;
        const resultsWriter = fs.createWriteStream(resultsCsv);

        // Write the header to the results CSV
        resultsWriter.write('File Location,Count\n');

        // Read the CSV file and count the file locations
        const csvData = fs.readFileSync(filePath, 'utf8').split('\n');
        csvData.forEach((row) => {
            const trimmedRow = row.trim(); // Remove leading/trailing whitespace
            if (trimmedRow !== '') { // Ignore empty lines
                const fileLocation = trimmedRow.split(',')[0];
                if (fileCounts[fileLocation]) {
                    fileCounts[fileLocation]++;
                } else {
                    fileCounts[fileLocation] = 1;
                }
            }
        });

        // Write the file location counts to the results CSV
        Object.keys(fileCounts).forEach((fileLocation) => {
            resultsWriter.write(`${fileLocation},${fileCounts[fileLocation]}\n`);
        });
        resultsWriter.end();

        fileIndex++;
        setTimeout(processFile, 250); // Call processFile again after 250ms
    }

    processFile(); // Start processing files
}

// 4. compare the new count.csv files with the previous days. 
// here we are looking for:
// - change in count for existing files 
// - new file locations included in count 
function compareCountFiles() {
    // define the folders to compare
    const currentDate = moment().format('MM-DD-YY');
    const previousDate = moment().subtract(1, 'day').format('MM-DD-YY');

    const currentDirectory = `./tallies/${currentDate}/count`;
    const previousDirectory = `./tallies/${previousDate}/count`;

    if (!fs.existsSync(previousDirectory)) {
        console.log('Previous day\'s directory does not exist. Cannot compare.');
        return;
    }

    const fileNames = [
        'count_dao-contracts_contracts_results.csv',
        'count_dao-contracts_packages_results.csv',
        'count_dao-contracts_scripts_results.csv',
    ];

    const resultsDirectory = `./tallies/${currentDate}/results`;
    if (!fs.existsSync(resultsDirectory)) {
        fs.mkdirSync(resultsDirectory);
    }

    let fileIndex = 0;
    function compareTallyChanges() {
        if (fileIndex >= fileNames.length) {
            return;
        }

        const fileName = fileNames[fileIndex];
        const currentFilePath = `${currentDirectory}/${fileName}`;
        const previousFilePath = `${previousDirectory}/${fileName}`;
        const resultFilePath = `${resultsDirectory}/${fileName.replace('count_dao-contracts', 'count_comparison')}`;

        if (!fs.existsSync(currentFilePath)) {
            console.log(`Current file ${fileName} does not exist. Skipping.`);
            fileIndex++;
            compareTallyChanges();
            return;
        }

        let writeData = 'file/location,previous count,current count,change\n';
        if (!fs.existsSync(previousFilePath)) {
            console.log(`New file ${fileName} added in the current day's directory.`);
            const currentFileData = fs.readFileSync(currentFilePath, 'utf8').split('\n');
            currentFileData.forEach((row, index) => {
                if (index > 0) {
                    const rowValues = row.split(',');
                    if (rowValues.length === 2) {
                        writeData += `${rowValues[0]},,${rowValues[1]},New file added\n`;
                    }
                }
            });
        } else {
            const currentFileData = fs.readFileSync(currentFilePath, 'utf8').split('\n');
            const previousFileData = fs.readFileSync(previousFilePath, 'utf8').split('\n');
            const previousFileDataMap = {};
            previousFileData.forEach((row) => {
                const rowValues = row.split(',');
                if (rowValues.length === 2) {
                    previousFileDataMap[rowValues[0]] = parseInt(rowValues[1]);
                }
            });

            currentFileData.forEach((row) => {
                const rowValues = row.split(',');
                if (rowValues.length === 2) {
                    const currentCount = parseInt(rowValues[1]);
                    const previousCount = previousFileDataMap[rowValues[0]];
                    if (previousCount !== undefined) {
                        const change = currentCount - previousCount;
                        if (change !== 0) {
                            writeData += `${rowValues[0]},${previousCount},${currentCount},${change}\n`;
                        } else {
                            writeData += `${rowValues[0]},${previousCount},${currentCount},No change\n`;
                        }
                    } else {
                        writeData += `${rowValues[0]},,${rowValues[1]},New file added\n`;
                    }
                }
            });
        }

        fs.writeFileSync(resultFilePath, writeData);
        console.log(`Result for ${fileName} has been written to ${resultFilePath}`);

        fileIndex++;
        compareTallyChanges();
    }

    compareTallyChanges();
}


/////////////////////////  MAIN FUNCTIONS ///////////////////////////////////
async function main() {
    try {
        await cloneRepository('https://github.com/DA0-DA0/dao-contracts', './dao-contracts');

        // 2. count, and print into tally-csv for each defined directory with delay between each call
        const promises = dirs.reduce((promiseChain, currentDir) => {
            return promiseChain.then(() => new Promise((resolve) => {
                runBashScript(currentDir);
                setTimeout(() => resolve(), 250); // wait 250ms before running the next script
            }));
        }, Promise.resolve());

        // 3. condense tally for each file into single value for next step
        await promises.then(() => {
            return new Promise((resolve) => {
                countFileLocations();
                setTimeout(() => resolve(), 250); // wait 250ms before running the next script
            });
        });
        // 4. compare the new count.csv files with the previous days. 
        compareCountFiles();
    } catch (error) {
        console.error('Error occurred:', error);
    }
}

main();