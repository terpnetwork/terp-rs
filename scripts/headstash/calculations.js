
import { readCsvFile, readYamlFile, toMarkdownTable, escapeRegExp, toOverviewMarkdownTable, loadProjectAddresses } from "./utils.js";
import readline from 'readline';
import { HEADSTASH_YAML, BASE_ALLOCATION, OVERVIEW_README } from './constants.js'
import fs from 'fs';
import path, { dirname } from 'path'
import { parse, stringify } from 'yaml'
import { fileURLToPath } from 'url';


const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);


// Helper to prompt input
const ask = (query) => {
    const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout,
    });
    return new Promise((resolve) => rl.question(query, (ans) => {
        rl.close();
        resolve(ans.trim());
    }));
};

/// prompts user to determine fair % ranges for 1,2,3 points
export const fairPercentileRanges = async (distributionData) => {
    const projects = await Promise.all(
        Object.values(distributionData.projects).map(async (proj) => {
            const records = await loadProjectAddresses(proj.csv);
            const holders = records
                .map((r) => ({
                    address: r.addr,
                    amount: parseFloat(r.amount),
                }))
                .filter((h) => !isNaN(h.amount));

            holders.sort((a, b) => b.amount - a.amount); // descending

            return { ...proj, holders };
        })
    );

    console.log("✅ loaded projects:", projects.map((p) => p.name));

    // Generate 3% increments: 3%, 6%, ..., 99%
    const percentiles = [];
    for (let p = 0.01; p <= 1; p += 0.01) {
        percentiles.push(parseFloat(p.toFixed(2)));
    }

    const configResults = {};

    // Process each project
    for (const proj of projects) {
        const { holders, name } = proj;
        if (holders.length === 0) {
            console.warn(`⚠️ No valid holder data for project: ${name}`);
            continue;
        }

        const total = holders.length;
        const percentileValues = {};

        // Build data for table
        for (const p of percentiles) {
            const idx = Math.floor(p * total);
            if (idx >= total) continue;
            const rank = idx + 1;
            const { amount } = holders[idx];
            const label = `${Math.round(p * 100)}%`;
            percentileValues[label] = {
                rank,
                totalHolders: total,
                requiredAmount: amount
            };
        }

        // print percentile range md table to readme of project folder 
        // ✅ Print the nice table you liked. will be in same folder as csv file
        console.log(`\n📊 ${name} - percentile ranges:`);
        console.table(percentileValues);

        const readmePath = path.join(__dirname, '..', 'headstash', 'communities', name, 'README.md');
        const newTable = toMarkdownTable(percentileValues);
        const header = `## ${name} - Percentile Ranges`;
        // Read and update README.md
        let readmeContent = '';
        if (fs.existsSync(readmePath)) {
            readmeContent = fs.readFileSync(readmePath, 'utf8');
        }

        // Regex to match from `## ...` to the next heading (or end of file)
        const regex = new RegExp(`(^|\\n)${escapeRegExp(header)}\\s*\\n[^\\n]*(.*?)(?=\\n## |\\n\\s*\\n|$)`, 's');
        const replacement = `\n${header}\n\n${newTable}`;
        const updatedContent = readmeContent.match(regex)
            ? readmeContent.replace(regex, replacement)
            : readmeContent + `\n${header}\n\n${newTable}\n`;

        // Ensure directory exists
        fs.mkdirSync(path.dirname(readmePath), { recursive: true });
        fs.writeFileSync(readmePath, updatedContent, 'utf8');



        // Interactive cutoff configuration
        console.log(`\n🎯 Now setting point tiers for ${name}...`);

        let threePerc;
        while (!threePerc) {
            const input = await ask(`   🥇 Top __% get 3 points? (1-99): `);
            const n = parseFloat(input);
            if (isNaN(n) || !Number.isInteger(n) || n < 1 || n > 99) {
                console.log(`   ❌ Invalid. Please enter a whole number between 1 and 99.`);
                continue;
            }
            const decimalValue = n / 100;
            if (percentiles.includes(decimalValue)) {
                threePerc = decimalValue;
            } else {
                console.log(`   ❌ ${n}% is not supported.`);
            }
        }

        let twoPerc;
        while (!twoPerc) {
            const input = await ask(`   🥈 Extend 2 points up to __%? (must be > ${threePerc * 100}%): `);
            const n = parseFloat(input);
            if (isNaN(n) || !Number.isInteger(n) || n < 1 || n > 99) {
                console.log(`   ❌ Must be a whole number between 1 and 99.`);
                continue;
            }
            const decimalValue = n / 100;
            if (decimalValue <= threePerc) {
                console.log(`   ❌ Must be greater than ${threePerc * 100}.`);
                continue;
            }
            if (!percentiles.includes(decimalValue)) {
                console.log(`   ❌ ${n}% is not supported.`);
                continue;
            }
            twoPerc = decimalValue;
        }

        // Calculate number of holders in each point tier
        const threePointCutoffIdx = Math.floor(threePerc * total);
        const twoPointCutoffIdx = Math.floor(twoPerc * total);

        const numThreePointHolders = threePointCutoffIdx + 1; // +1 because 0-indexed
        const numTwoPointHolders = twoPointCutoffIdx - threePointCutoffIdx;
        const numOnePointHolders = total - twoPointCutoffIdx - 1;
        // Inside fairPercentileRanges(), after:

        // Save the actual token amount thresholds
        const threePointCutoffAmount = holders[threePointCutoffIdx]?.amount;
        const twoPointCutoffAmount = holders[twoPointCutoffIdx]?.amount;


        // Save configuration
        configResults[name] = {
            threePointsUpTo: {
                percentile: threePerc,
                cutoffAmount: threePointCutoffAmount,
                holders: numThreePointHolders
            },
            twoPointsUpTo: {
                percentile: twoPerc,
                cutoffAmount: twoPointCutoffAmount,
                holders: numTwoPointHolders
            },
            onePointUpTo: {
                percentile: 1.0,
                cutoffAmount: 0,
                holders: numOnePointHolders
            },
            totalHolders: total,
        };

        await updateHeadstashYaml(configResults, name);
        console.log(` ✅ ${name} configured: 3pts ≤ ${threePerc * 100}%, 2pts ≤ ${twoPerc * 100}%, 1pt rest\n`);
    }

    // Final result
    console.log("📋 Full configuration results:");

    console.log(configResults);
    return;
};


export const updateHeadstashYaml = async (configResults, name) => {
    const doc = await readYamlFile(HEADSTASH_YAML)

    // Find the specific project by name in configResults
    const config = configResults[name];
    if (!config) {
        console.warn(` ⚠️ Project "${name}" not found in config results`);
        return;
    }

    // Find project by name in the YAML under `projects`
    const project = doc.projects?.find(p => p.name === name);
    if (project) {
        project.points = {
            threePointsUpTo: config.threePointsUpTo,
            twoPointsUpTo: config.twoPointsUpTo,
            onePointUpTo: config.onePointUpTo,
            totalHolders: config.totalHolders
        };
        console.log(` 📥 Updated ${name} points distribution in headstash.yaml`);
    } else {
        console.warn(` ⚠️ Project "${name}" not found in headstash.yaml`);
    }
    fs.writeFileSync(HEADSTASH_YAML, stringify(doc), 'utf8');
    console.log(`✅ headstash.yaml updated with new points distribution for "${name}"`);
};

/**
 * Calculate project points using square root scaling
 * @param {Array} projects - [{ name, totalHolders }]
 * @param {number} baseAllocation - Total points to distribute
 * @returns {Object} { projectName: points }
 */
function calculateSqrtPoints(projects, baseAllocation) {
    const sumSqrt = projects.reduce((sum, p) => sum + Math.sqrt(p.totalHolders), 0);
    if (sumSqrt === 0) return {};

    return projects.reduce((acc, p) => {
        acc[p.name] = Math.round((Math.sqrt(p.totalHolders) / sumSqrt) * baseAllocation);
        return acc;
    }, {});
}

/**
 * Calculate project points using logarithmic scaling
 * @param {Array} projects - [{ name, totalHolders }]
 * @param {number} baseAllocation - Total points to distribute
 * @returns {Object} { projectName: points }
 */
function calculateLogPoints(projects, baseAllocation) {
    const sumLog = projects.reduce((sum, p) => sum + Math.log(p.totalHolders + 1), 0);
    if (sumLog === 0) return {};

    return projects.reduce((acc, p) => {
        acc[p.name] = Math.round((Math.log(p.totalHolders + 1) / sumLog) * baseAllocation);
        return acc;
    }, {});
}


/**
 * Recalculates and updates normalization points for ALL projects in headstash.yaml
 * Uses existing `totalHolders` from each project's `points` field
 */
export const applyNormalizationToAllProjects = async () => {
    const doc = await readYamlFile(HEADSTASH_YAML);

    if (!doc.projects || !Array.isArray(doc.projects)) {
        console.warn('⚠️ No projects found in headstash.yaml');
        return;
    }

    // Extract totalHolders from existing points data
    const projectsData = doc.projects
        .filter(p => p.points?.totalHolders !== undefined)
        .map(p => ({
            name: p.name,
            totalHolders: p.points.totalHolders,
        }));

    if (projectsData.length === 0) {
        console.warn('⚠️ No valid project holder data found for normalization');
        return;
    }
    internalWriteHeadstashYaml(doc, projectsData)
};



export const internalWriteHeadstashYaml = async (doc, projectsData) => {
    // Reuse the same helper functions
    const sqrtAllocations = calculateSqrtPoints(projectsData, BASE_ALLOCATION);
    const logAllocations = calculateLogPoints(projectsData, BASE_ALLOCATION);

    // Update every project in the YAML
    doc.projects.forEach(project => {
        const holders = project.points?.totalHolders;
        if (holders === undefined) return;

        // Ensure points object exists
        if (!project.points) project.points = {};

        project.points.normalization = {
            sqrtScaling: sqrtAllocations[project.name] || 0,
            logScaling: logAllocations[project.name] || 0,
        };
        // allocation %
        project.allocation_percentage = project.points.normalization.sqrtScaling / BASE_ALLOCATION
    });

    // Update every project in the YAML
    let totalPointsInAllProjects = 0;
    doc.projects.forEach(project => {
        const p = project.points;


        // Safely parse and clamp ranges
        const totalHolders = p.totalHolders;
        const threePointsUpTo = p.threePointsUpTo.holders || 0;
        const twoPointsUpTo = Math.max(threePointsUpTo, p.twoPointsUpTo.holders || 0);
        const onePointUpTo = Math.max(twoPointsUpTo, p.onePointUpTo.holders || 0);

        // Count holders in each tier
        const rank3 = Math.min(threePointsUpTo, totalHolders);
        const rank2 = Math.min(twoPointsUpTo, totalHolders) - rank3;
        const rank1 = Math.min(onePointUpTo, totalHolders) - rank3 - rank2;

        const projectTotalPoints = rank3 * 3 + rank2 * 2 + rank1 * 1;
        totalPointsInAllProjects += projectTotalPoints;
    });

    // === Prevent NaN: validate total points ===
    if (totalPointsInAllProjects <= 0) {
        console.error('❌ Total points across projects is zero or invalid. Cannot compute TPP.');
        console.log('👉 Check: do your projects have valid threePointsUpTo, twoPointsUpTo, onePointUpTo?');
        return;
    }

    const tokensPerPoint = BASE_ALLOCATION / totalPointsInAllProjects;

    // Final pass: assign allocation_percentage and tpp
    doc.projects.forEach(project => {
        if (!project.points?.normalization) return;

        // Safe allocation percentage
        const sqrtScaling = project.points.normalization.sqrtScaling || 0;
        project.allocation_percentage = sqrtScaling / BASE_ALLOCATION;

        console.log(tokensPerPoint)
        // Assign global TPP
        project.tpp = Number(tokensPerPoint.toFixed(8));
    });
    fs.writeFileSync(HEADSTASH_YAML, stringify(doc), 'utf8');
    console.log('Normalization applied to all projects in headstash.yaml');
}


export async function generateOverviewReadme() {
    try {
        const data = await readYamlFile(HEADSTASH_YAML);

        if (!data?.projects?.length) {
            console.error('❌ No projects found in YAML.');
            return;
        }

        const table = toOverviewMarkdownTable(data.projects);
        const sectionHeader = '## Airdrop Cycle 2: Cannabis Culture Communities';

        let mdContent = '';
        if (fs.existsSync(OVERVIEW_README)) {
            mdContent = fs.readFileSync(OVERVIEW_README, 'utf8');
        }

        // Regex to match from `## Airdrop Cycle 2...` to the next heading of level 2 or higher (`## ` or start of file)
        const regex = new RegExp(`(^|\\n)(${escapeRegExp(sectionHeader)}\\s*\\n)([^\\n]*(?:\\n(?!## )[^\\n]*)*)(?=\\n## |$)`, 's');
        const replacement = `\n${sectionHeader}\n\n${table}`;
        const updatedContent = regex.test(mdContent)
            ? mdContent.replace(regex, replacement)
            : mdContent + `\n${sectionHeader}\n\n${table}\n`;

        fs.writeFileSync(OVERVIEW_README, updatedContent, 'utf8');
        console.log('✅ Markdown table generated and injected into', OVERVIEW_README);
    } catch (err) {
        console.error('❌ Error generating overview:', err.message);
    }
};

// validate:
// community points summary reflects what is in headstash.yaml
// all projects allocation % totals up to 100%
// tokens per point * total tokens matches projects expected token allocations

// full headstash sequence
// - use exsisting, or create default yaml file, based on each project folder in headstash/communities
//  - process any solana project
// - determine percentile ranges for each project, print md tables into projects folder README.
// - determine scaling factors, resulting in complete yaml file
// - create final-output.csv & final summary MD, printed to 