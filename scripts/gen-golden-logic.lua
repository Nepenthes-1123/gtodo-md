-- タスクの並べ替え・期日による昇格のゴールデンデータを、Lua 版 (gtodo-md.nvim) の
-- logic/ から生成する。
--
-- 使い方:
--   nvim --clean --headless -l scripts/gen-golden-logic.lua <gtodo-md.nvim のパス>
--
-- 入力:  testdata/golden/logic/cases.json と、ケースごとのディレクトリ (inbox.md / todo.md / done.md)
-- 出力:  testdata/golden/logic.json (ケースごとに、書き込まれたファイルと行を書き込んだ順に)
--
-- `--clean` が必須な理由は gen-golden-task.lua と同じ。ファイルは書き換えない (書き込みは横取りする)。

local plugin_dir = arg[1]
if not plugin_dir or plugin_dir == "" then
	io.stderr:write("usage: nvim --clean --headless -l scripts/gen-golden-logic.lua <path/to/gtodo-md.nvim>\n")
	os.exit(2)
end
package.path = table.concat({ plugin_dir .. "/lua/?.lua", plugin_dir .. "/lua/?/init.lua", package.path }, ";")

-- 正午にする理由は gen-golden-task.lua と同じ。
local TODAY = { year = 2026, month = 2, day = 3 }
local real_time, real_date = os.time, os.date
local fixed_now = real_time({ year = TODAY.year, month = TODAY.month, day = TODAY.day, hour = 12 })
os.time = function(t)
	if t == nil then
		return fixed_now
	end
	return real_time(t)
end
os.date = function(fmt, t)
	return real_date(fmt, t or fixed_now)
end

local config = require("gtodo-md.config")
local io_mod = require("gtodo-md.io")
local task_mod = require("gtodo-md.task")
local logic = require("gtodo-md.logic")

-- 新しく発行される ID はどの ID とも重ならない連番にする (検証側は入力に無い ID を出現順の番号に置き換えて比べる)。
local next_id = 0
task_mod._generate_id = function()
	next_id = next_id + 1
	return string.format("new%03d", next_id)
end

local writes
io_mod.write_lines = function(path, lines)
	table.insert(writes, { file = vim.fs.basename(path), lines = vim.deepcopy(lines) })
end
vim.notify = function() end

local root = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(vim.fn.fnamemodify(arg[0], ":p"))))
local case_root = root .. "/testdata/golden/logic"
local out_path = root .. "/testdata/golden/logic.json"
local cases = vim.json.decode(table.concat(vim.fn.readfile(case_root .. "/cases.json"), "\n"))

local function encode_string_list(list)
	local parts = {}
	for _, s in ipairs(list) do
		table.insert(parts, vim.json.encode(s))
	end
	return "[" .. table.concat(parts, ",") .. "]"
end

local function encode_map(map)
	local keys = vim.tbl_keys(map or {})
	table.sort(keys)
	local parts = {}
	for _, k in ipairs(keys) do
		table.insert(parts, vim.json.encode(k) .. ":" .. vim.json.encode(map[k]))
	end
	return "{" .. table.concat(parts, ",") .. "}"
end

local out = { "{", string.format('  "today": "%04d-%02d-%02d",', TODAY.year, TODAY.month, TODAY.day), '  "cases": [' }
for i, case in ipairs(cases) do
	local dir = case_root .. "/" .. case.dir
	local auto_move = case.auto_move_inbox_to_today ~= false
	config.options = {
		data_dir = dir,
		auto_move_inbox_to_today = auto_move,
		due_notification_persist = false,
	}
	config.sections = vim.tbl_extend("force", {}, config.default_sections, case.sections or {})
	config.last_sections = {}

	writes = {}
	local inbox, todo = dir .. "/inbox.md", dir .. "/todo.md"
	if case.op == "sort" then
		logic.sort_todo_file(todo)
	elseif case.op == "check_dues" then
		logic.check_dues(inbox, todo)
	else
		error("unknown op: " .. tostring(case.op))
	end

	local encoded = {}
	for _, w in ipairs(writes) do
		table.insert(encoded, string.format('{"file":%s,"lines":%s}', vim.json.encode(w.file), encode_string_list(w.lines)))
	end
	local entry = string.format(
		'{"dir":%s,"op":%s,"auto_move_inbox_to_today":%s,"sections_config":%s,"writes":[%s]}',
		vim.json.encode(case.dir),
		vim.json.encode(case.op),
		tostring(auto_move),
		encode_map(case.sections),
		table.concat(encoded, ",")
	)
	table.insert(out, "    " .. entry .. (i < #cases and "," or ""))
end
table.insert(out, "  ]")
table.insert(out, "}")

vim.fn.writefile(out, out_path)
print(string.format("wrote %d cases to %s", #cases, out_path))
