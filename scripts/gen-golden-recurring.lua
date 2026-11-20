-- 定期タスク (テンプレートのフロントマター) のゴールデンデータを、Lua 版 (gtodo-md.nvim) の
-- logic/recurring.lua (規則の読み取り・判定) と recurring.lua (inbox.md へ足す処理) から生成する。
--
-- 使い方:
--   nvim --clean --headless -l scripts/gen-golden-recurring.lua <gtodo-md.nvim のパス>
--
-- 入力:  testdata/golden/recurring/cases.json
-- 出力:  testdata/golden/recurring.json
--
-- `--clean` が必須な理由は gen-golden-task.lua と同じ。足す処理のケースは、ケースごとに一時ディレクトリを
-- データディレクトリにして実行する (入力のディレクトリには触れない)。

local plugin_dir = arg[1]
if not plugin_dir or plugin_dir == "" then
	io.stderr:write("usage: nvim --clean --headless -l scripts/gen-golden-recurring.lua <path/to/gtodo-md.nvim>\n")
	os.exit(2)
end
package.path = table.concat({ plugin_dir .. "/lua/?.lua", plugin_dir .. "/lua/?/init.lua", package.path }, ";")

local logic = require("gtodo-md.logic.recurring")

local root = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(vim.fn.fnamemodify(arg[0], ":p"))))
local cases_path = root .. "/testdata/golden/recurring/cases.json"
local out_path = root .. "/testdata/golden/recurring.json"
local cases = vim.json.decode(table.concat(vim.fn.readfile(cases_path), "\n"), { luanil = { object = true } })

-- vim.json.encode はキーの順序を保証しないため、差分が安定するよう自前で並べる。
local function encode_object(keys, t)
	local parts = {}
	for _, k in ipairs(keys) do
		if t[k] ~= nil then
			table.insert(parts, vim.json.encode(k) .. ":" .. vim.json.encode(t[k]))
		end
	end
	return "{" .. table.concat(parts, ",") .. "}"
end

local function encode_sorted(t)
	local keys = vim.tbl_keys(t)
	table.sort(keys)
	return encode_object(keys, t)
end

local function encode_list(items, encode)
	local parts = {}
	for _, item in ipairs(items) do
		table.insert(parts, encode(item))
	end
	return "[" .. table.concat(parts, ",") .. "]"
end

local RULE_KEYS = { "kind", "nth", "weekday", "month", "day" }

local function encode_rules(rules)
	return encode_list(rules, function(r)
		return encode_object(RULE_KEYS, r)
	end)
end

local out = { "{" }
local function add(line)
	table.insert(out, line)
end

-- repeat の読み取り。読めなければ "error"。
add('  "repeat": [')
for i, spec in ipairs(cases["repeat"]) do
	local rules = logic.parse_repeat(spec)
	add(
		string.format(
			'    {"spec":%s,"rules":%s}%s',
			vim.json.encode(spec),
			rules and encode_rules(rules) or '"error"',
			i < #cases["repeat"] and "," or ""
		)
	)
end
add("  ],")

-- 期間の中で当たる日。
local from = logic._parse_date(cases.occurs.from)
local to = logic._parse_date(cases.occurs.to)
add(string.format('  "occurs_from": %s,', vim.json.encode(cases.occurs.from)))
add(string.format('  "occurs_to": %s,', vim.json.encode(cases.occurs.to)))
add('  "occurs": [')
for i, spec in ipairs(cases.occurs.specs) do
	local rules = assert(logic.parse_repeat(spec), spec)
	local dates = {}
	for z = from, to do
		local date = logic._format_date(z)
		if logic.occurs_on(rules, date) then
			table.insert(dates, date)
		end
	end
	add(
		string.format(
			'    {"spec":%s,"dates":%s}%s',
			vim.json.encode(spec),
			vim.json.encode(dates),
			i < #cases.occurs.specs and "," or ""
		)
	)
end
add("  ],")

add('  "latest": [')
for i, c in ipairs(cases.latest) do
	local result = logic.latest_occurrence(assert(logic.parse_repeat(c.spec)), c.after, c.today)
	add(
		string.format(
			'    {"spec":%s,"after":%s,"today":%s,"result":%s}%s',
			vim.json.encode(c.spec),
			c.after and vim.json.encode(c.after) or "null",
			vim.json.encode(c.today),
			result and vim.json.encode(result) or "null",
			i < #cases.latest and "," or ""
		)
	)
end
add("  ],")

add('  "next": [')
for i, c in ipairs(cases.next) do
	local result = logic.next_occurrence(assert(logic.parse_repeat(c.spec)), c.from)
	add(
		string.format(
			'    {"spec":%s,"from":%s,"result":%s}%s',
			vim.json.encode(c.spec),
			vim.json.encode(c.from),
			result and vim.json.encode(result) or "null",
			i < #cases.next and "," or ""
		)
	)
end
add("  ],")

-- フロントマターの読み取り。エラーの文言はツールごとに違ってよいので、あるかどうかだけを残す。
add('  "definitions": [')
for i, lines in ipairs(cases.definitions) do
	local def = logic.read_definition(lines)
	add(
		string.format(
			'    {"lines":%s,"recurring":%s,"rules":%s,"repeat_text":%s,"values":%s,"body":%s,"error":%s}%s',
			vim.json.encode(lines),
			tostring(def.recurring),
			def.rules and encode_rules(def.rules) or "null",
			def.repeat_text and vim.json.encode(def.repeat_text) or "null",
			encode_sorted(def.values),
			vim.json.encode(def.body),
			tostring(def.error ~= nil),
			i < #cases.definitions and "," or ""
		)
	)
end
add("  ],")

-- inbox.md へ足す処理。足したタスク行 (ID を除く)・記録・飛ばしたテンプレートの名前。
local config = require("gtodo-md.config")
local state = require("gtodo-md.state")
local recurring = require("gtodo-md.recurring")
local notices
vim.notify = function(msg)
	table.insert(notices, msg)
end

local INBOX = { "# Inbox", "", "- [ ] 既存のタスク id:aaaaaa" }

add('  "inbox": ' .. vim.json.encode(INBOX) .. ",")
add('  "create": [')
for i, c in ipairs(cases.create) do
	local data_dir = vim.fn.tempname()
	vim.fn.mkdir(data_dir .. "/templates", "p")
	config.setup({ data_dir = data_dir })
	vim.fn.writefile(INBOX, data_dir .. "/inbox.md")
	for name, lines in pairs(c.templates) do
		vim.fn.writefile(lines, data_dir .. "/templates/" .. name .. ".md")
	end
	if next(c.recurring) ~= nil then
		assert(state.write_recurring(c.recurring))
	end
	notices = {}
	local count = recurring.create_due_tasks(c.today)

	local added = {}
	for n, line in ipairs(vim.fn.readfile(data_dir .. "/inbox.md")) do
		if n > #INBOX then
			table.insert(added, (line:gsub(" id:%x+$", "")))
		end
	end
	local skipped = {}
	for _, msg in ipairs(notices) do
		if msg:find("were skipped", 1, true) then
			for line in msg:gmatch("\n([^\n]+)") do
				table.insert(skipped, line:match("^([^:]+):"))
			end
		end
	end
	assert(#added == count, c.name)
	add(
		string.format(
			'    {"name":%s,"added":%s,"records":%s,"skipped":%s}%s',
			vim.json.encode(c.name),
			vim.json.encode(added),
			encode_sorted(state.read_recurring()),
			vim.json.encode(skipped),
			i < #cases.create and "," or ""
		)
	)
	vim.fn.delete(data_dir, "rf")
end
add("  ]")
add("}")

vim.fn.writefile(out, out_path)
print("wrote " .. out_path)
