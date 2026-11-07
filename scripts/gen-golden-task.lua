-- タスク行の文法のゴールデンデータを、Lua 版 (gtodo-md.nvim) の task.parse / task.serialize から生成する。
--
-- 使い方:
--   nvim --clean --headless -l scripts/gen-golden-task.lua <gtodo-md.nvim のパス>
--
-- 入力:  testdata/golden/task_cases.json (タスク行の文字列の配列)
-- 出力:  testdata/golden/task_parse.json
--
-- `--clean` は必須。task.parse の `\<` はカレントバッファの 'iskeyword' に依存するため、
-- ユーザー設定が混ざると既定と異なる結果が記録される。

local plugin_dir = arg[1]
if not plugin_dir or plugin_dir == "" then
	io.stderr:write("usage: nvim --clean --headless -l scripts/gen-golden-task.lua <path/to/gtodo-md.nvim>\n")
	os.exit(2)
end
package.path = table.concat({ plugin_dir .. "/lua/?.lua", plugin_dir .. "/lua/?/init.lua", package.path }, ";")

-- 期日の相対指定(+1m, sat 等)は「今日」から解決されるため、基準日を固定する。
-- 正午にしているのは、Lua 版が秒の加算で日付を進めるため、日付の境界付近だと
-- 夏時間のあるタイムゾーンで結果がずれるのを避けるため。
-- 2026-01-31(土)は、月末クランプ(+1m → 2/28)と同じ曜日の指定(sat → 翌週)を同時に検査できる。
local TODAY = { year = 2026, month = 1, day = 31 }
local real_time = os.time
local fixed_now = real_time({ year = TODAY.year, month = TODAY.month, day = TODAY.day, hour = 12 })
os.time = function(t)
	if t == nil then
		return fixed_now
	end
	return real_time(t)
end

local task_mod = require("gtodo-md.task")

local root = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(vim.fn.fnamemodify(arg[0], ":p"))))
local cases_path = root .. "/testdata/golden/task_cases.json"
local out_path = root .. "/testdata/golden/task_parse.json"

local cases = vim.json.decode(table.concat(vim.fn.readfile(cases_path), "\n"))

local FIELDS = {
	"indent",
	"status",
	"priority",
	"content",
	"project",
	"context",
	"due",
	"created",
	"wait",
	"completed_at",
	"done",
	"cancelled",
	"from",
	"id",
}

-- vim.json.encode はキーの順序を保証しないため、差分が安定するよう自前で並べる。
local function encode_task(t)
	local parts = {}
	for _, k in ipairs(FIELDS) do
		if t[k] ~= nil then
			table.insert(parts, vim.json.encode(k) .. ":" .. vim.json.encode(t[k]))
		end
	end
	return "{" .. table.concat(parts, ",") .. "}"
end

local lines = { "{" }
table.insert(
	lines,
	string.format('  "today": "%04d-%02d-%02d",', TODAY.year, TODAY.month, TODAY.day)
)
-- serialize は id が無いと乱数で発行するため、出力を決定的にするよう固定値を補う。
table.insert(lines, '  "placeholder_id": "000000",')
table.insert(lines, '  "cases": [')
for i, input in ipairs(cases) do
	local parsed = task_mod.parse(input)
	local entry
	if parsed then
		local fields = encode_task(parsed)
		if not parsed.id or parsed.id == "" then
			parsed.id = "000000"
		end
		local serialized = task_mod.serialize(parsed)
		entry = string.format(
			'{"input":%s,"parsed":%s,"serialized":%s}',
			vim.json.encode(input),
			fields,
			vim.json.encode(serialized)
		)
	else
		entry = string.format('{"input":%s,"parsed":null}', vim.json.encode(input))
	end
	table.insert(lines, "    " .. entry .. (i < #cases and "," or ""))
end
table.insert(lines, "  ]")
table.insert(lines, "}")

vim.fn.writefile(lines, out_path)
print(string.format("wrote %d cases to %s", #cases, out_path))
