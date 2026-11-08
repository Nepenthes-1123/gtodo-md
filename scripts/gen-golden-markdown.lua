-- ファイル構造の解析 (io.parse_markdown) と書き出し (io.write_todo_file) のゴールデンデータを、
-- Lua 版 (gtodo-md.nvim) から生成する。
--
-- 使い方:
--   nvim --clean --headless -l scripts/gen-golden-markdown.lua <gtodo-md.nvim のパス>
--
-- 入力:  testdata/golden/markdown/cases.json と、そこから参照する *.md
-- 出力:  testdata/golden/markdown_parse.json (解析結果と、それを書き出した行)
--
-- `--clean` が必須な理由は gen-golden-task.lua と同じ。

local plugin_dir = arg[1]
if not plugin_dir or plugin_dir == "" then
	io.stderr:write("usage: nvim --clean --headless -l scripts/gen-golden-markdown.lua <path/to/gtodo-md.nvim>\n")
	os.exit(2)
end
package.path = table.concat({ plugin_dir .. "/lua/?.lua", plugin_dir .. "/lua/?/init.lua", package.path }, ";")

-- タスク行の相対期日の解決を決定的にするため、gen-golden-task.lua と同じ基準日に固定する。
local TODAY = { year = 2026, month = 1, day = 31 }
local real_time = os.time
local fixed_now = real_time({ year = TODAY.year, month = TODAY.month, day = TODAY.day, hour = 12 })
os.time = function(t)
	if t == nil then
		return fixed_now
	end
	return real_time(t)
end

local config = require("gtodo-md.config")
local io_mod = require("gtodo-md.io")
local task_mod = require("gtodo-md.task")

-- 新しく発行される ID はランダムなので、どの ID とも重ならない連番にする
-- (検証側は、入力に無い ID をすべて同じ印に置き換えて比べる)。
local next_id = 0
task_mod._generate_id = function()
	next_id = next_id + 1
	return string.format("new%03d", next_id)
end

-- write_todo_file がディスクへ渡す行を受け取る。
local written
io_mod.write_lines = function(_, lines)
	written = lines
end

local root = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(vim.fn.fnamemodify(arg[0], ":p"))))
local case_dir = root .. "/testdata/golden/markdown"
local out_path = root .. "/testdata/golden/markdown_parse.json"

local cases = vim.json.decode(table.concat(vim.fn.readfile(case_dir .. "/cases.json"), "\n"))

local function encode_items(items)
	local parts = {}
	for _, item in ipairs(items) do
		local kind = item.type == "task" and "task" or "text"
		table.insert(parts, string.format('{"kind":"%s","line":%s}', kind, vim.json.encode(item.line)))
	end
	return "[" .. table.concat(parts, ",") .. "]"
end

local function encode_string_list(list)
	local parts = {}
	for _, s in ipairs(list) do
		table.insert(parts, vim.json.encode(s))
	end
	return "[" .. table.concat(parts, ",") .. "]"
end

-- キーの順序を固定して出力する (vim.json.encode は順序を保証しない)。
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
	config.sections = vim.tbl_extend("force", {}, config.default_sections, case.sections or {})
	config.last_sections = case.last_sections or {}

	-- read_lines は開いているバッファを優先するが、--clean の headless では
	-- 対象ファイルのバッファは無いため、ディスクから読まれる。
	local lines = io_mod.read_lines(case_dir .. "/" .. case.file)
	local data = io_mod.parse_markdown(lines)

	local sections = {}
	for _, name in ipairs(data.section_order) do
		table.insert(sections, string.format('{"name":%s,"items":%s}', vim.json.encode(name), encode_items(data.sections[name])))
	end

	-- Lua 版は最初の `##` より前のタスク・テキストを内部キー "default" のセクションに入れる。
	-- Rust 版はこれを preamble として別に持つ。
	local entry = string.format(
		'{"file":%s,"sections_config":%s,"last_sections":%s,"header":%s,"preamble":%s,"sections":[%s]}',
		vim.json.encode(case.file),
		encode_map(case.sections),
		encode_map(case.last_sections),
		encode_string_list(data.header),
		encode_items(data.sections["default"] or {}),
		table.concat(sections, ",")
	)
	-- write_todo_file は data のタスクに ID を書き込むため、解析結果を文字列にした後で呼ぶ。
	written = nil
	io_mod.write_todo_file(case_dir .. "/" .. case.file, data)
	assert(written, "write_todo_file no longer calls io.write_lines: " .. case.file)
	entry = entry:sub(1, -2) .. ',"written":' .. encode_string_list(written) .. "}"
	table.insert(out, "    " .. entry .. (i < #cases and "," or ""))
end
table.insert(out, "  ]")
table.insert(out, "}")

vim.fn.writefile(out, out_path)
print(string.format("wrote %d cases to %s", #cases, out_path))
