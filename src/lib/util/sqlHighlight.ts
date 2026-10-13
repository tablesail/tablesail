// Cheap SQL syntax colouring for read-only text (query log, assistant replies).
// Output is escaped HTML with <span class="k|s|n|c"> for keywords, strings, numbers, comments.

const KEYWORDS = new Set(
  (
    'SELECT FROM WHERE AND OR NOT NULL IS IN LIKE ILIKE BETWEEN EXISTS AS ON USING JOIN INNER LEFT RIGHT FULL OUTER CROSS NATURAL ' +
    'GROUP BY ORDER HAVING LIMIT OFFSET FETCH UNION ALL INTERSECT EXCEPT DISTINCT CASE WHEN THEN ELSE END ASC DESC ' +
    'INSERT INTO VALUES UPDATE SET DELETE RETURNING WITH RECURSIVE OVER PARTITION ' +
    'CREATE ALTER DROP TRUNCATE TABLE VIEW INDEX UNIQUE PRIMARY KEY FOREIGN REFERENCES DEFAULT CONSTRAINT CHECK ADD COLUMN RENAME TO ' +
    'BEGIN COMMIT ROLLBACK START TRANSACTION SAVEPOINT RELEASE USE SHOW DESCRIBE EXPLAIN ANALYZE PRAGMA ' +
    'TRUE FALSE IF CAST COLLATE ESCAPE FOR SESSION GLOBAL LOCAL KILL QUERY DEALLOCATE'
  ).split(' '),
)

// Groups: 1 comment, 2 string, 3 quoted name (left plain), 4 number, 5 word (keywords only).
const TOKEN =
  /(--[^\n]*|\/\*[\s\S]*?(?:\*\/|$))|('(?:[^'\\]|\\[\s\S]|'')*'?)|("(?:[^"]|"")*"?|`[^`]*`?)|(\b\d+(?:\.\d+)?\b)|([A-Za-z_][A-Za-z0-9_$]*)/g

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/** HTML for `sql` with keywords, strings, numbers and comments wrapped in coloured spans. */
export function highlightSql(sql: string): string {
  let out = ''
  let last = 0
  for (const m of sql.matchAll(TOKEN)) {
    const at = m.index ?? 0
    out += escapeHtml(sql.slice(last, at))
    last = at + m[0].length
    const text = escapeHtml(m[0])
    if (m[1] !== undefined) out += `<span class="c">${text}</span>`
    else if (m[2] !== undefined) out += `<span class="s">${text}</span>`
    else if (m[3] !== undefined) out += text
    else if (m[4] !== undefined) out += `<span class="n">${text}</span>`
    else if (KEYWORDS.has(m[0].toUpperCase())) out += `<span class="k">${text}</span>`
    else out += text
  }
  return out + escapeHtml(sql.slice(last))
}
