import fs from 'fs';
import csv from 'csv-parser';
import { parse, stringify } from 'yaml'

async function loadProjectAddresses(csvPath) {
    const rows = await readCsvFile(csvPath);
    // Assume CSV has headers: addr,amount
    return rows.map((r) => ({
        addr: r.addr,
        amount: r.amount
    }));
}


// Read Yaml file, parses into JSON object
const readYamlFile = async (filename) => {
    // Read existing YAML file
    const fileContent = fs.readFileSync(filename, 'utf8');
    return parse(fileContent);
};


// Read JSON files
const readJsonFile = async (filename) => {
    return new Promise((resolve, reject) => {
        fs.readFile(filename, 'utf8', (err, data) => {
            if (err) {
                reject(err);
            } else {
                resolve(JSON.parse(data));
            }
        });
    });
};

// Read CSV file
function readCsvFile(filePath) {
    return new Promise((resolve, reject) => {
        const csvData = [];

        fs.createReadStream(filePath)
            .pipe(csv())
            .on('data', (row) => { csvData.push(row) })
            .on('end', () => { resolve(csvData) })
            .on('error', (error) => { reject(error) });
    });
}


// Convert percentileValues to Markdown table
function toMarkdownTable(data) {
    if (!data || Object.keys(data).length === 0) return 'No data';

    let rows;

    if (Array.isArray(data)) {
        rows = data;
    } else {
        // Convert { "1%": { ... } } → [ { percentile: "1%", ... } ]
        rows = Object.entries(data).map(([key, value]) => ({
            percentile: key,
            ...value
        }));
    }

    const headers = Object.keys(rows[0]);
    const separator = headers.map(() => '---');

    return [
        '| ' + headers.join(' | ') + ' |',
        '| ' + separator.join(' | ') + ' |',
        ...rows.map(row => '| ' + headers.map(h => String(row[h] ?? '')).join(' | ') + ' |')
    ].join('\n');
}

function toOverviewMarkdownTable(projects) {
    const headers = [
        'Project',
        '# of Addresses',
        'Date Of Snapshot',
        'Est. Total TERP',
        'Average Token Per Point',
        '% of headstash allocation'
    ];
    const separator = ['---', '---', '---', '---', '---', '---'];

    // Sort projects by name for consistent output
    projects.sort((a, b) => a.name.localeCompare(b.name));

    const rows = projects.map(p => {
        const csvPath = p.csv.replace('../', './'); // Normalize path for URL
        const readmePath = csvPath.replace(/\/[^\/]+\.csv$/, '/README.md'); // Replace csv with README
        const projectLink = `[${p.name.replace(/-/g, ' ')}](${readmePath})`;

        // Format date: yy-mm-dd → MMM Do, YYYY (with proper ordinal suffixes)
        const [year, month, day] = p.snapshot_date.split('-');
        const date = new Date(`20${year}`, month - 1, parseInt(day, 10)); // Ensure day is int

        const formattedDate = isNaN(date.getTime())
            ? 'N/A'
            : date.toLocaleDateString('en-US', {
                year: 'numeric',
                month: 'short',
                day: 'numeric'
            }).replace(/\b(\d+)(?=\b)/, (match) => {
                const num = parseInt(match, 10);
                const suffix = ['th', 'st', 'nd', 'rd'][(num % 10) - 1] || 'th';
                // Handle teens (11th, 12th, 13th) — special case
                if (num > 10 && num < 20) return num + 'th';
                return num + suffix;
            });
        // Calculate est total TERP = tpp * totalHolders
        const estTotalTERP = p.tpp && p.points.totalHolders ? (p.tpp * p.points.totalHolders).toFixed(2) : '';

        // Avg Token Per Point = tpp
        const avgTPP = p.tpp ? p.tpp.toFixed(6) : '';

        // % of total supply
        const percentSupply = p.allocation_percentage ? `${(p.allocation_percentage * 100).toFixed(2)}%` : '';

        return [
            projectLink,
            `\`${p.points.totalHolders || 'N/A'}\``,
            formattedDate,
            estTotalTERP ? `\`${estTotalTERP} TERP\`` : '',
            `\`${avgTPP}\``,
            percentSupply ? `\`${percentSupply}\`` : ''
        ].join(' | ');
    });

    return [
        '| ' + headers.join(' | ') + ' |',
        '| ' + separator.join(' | ') + ' |',
        ...rows.map(row => '| ' + row + ' |'),
        '| |`' + projects.reduce((sum, p) => sum + p.points.totalHolders, 0) + '`| | `' + projects.reduce((sum, p) => sum + (p.tpp * p.points.totalHolders || 0), 0).toFixed(6) + ' TERP & THIOL` |||',
        ''
    ].join('\n');
}


function escapeRegExp(string) {
    return string.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); // Escape special regex chars
}


/* ------------------------------------------------------------
   Append a line to a CSV file (creates the file if it does not
   exist).  The file is opened in append mode for low‑overhead I/O.
   ------------------------------------------------------------ */
async function appendToCsv(filePath, values) {
    const line = stringify([values], { header: false });
    fs.appendFile(filePath, line);
}


export { readCsvFile, toOverviewMarkdownTable, readJsonFile, readYamlFile, toMarkdownTable, escapeRegExp, loadProjectAddresses, appendToCsv }

