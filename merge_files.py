import os
from datetime import datetime


def _get_branches(folder_path: str) -> tuple[list[str], str]:
    """
    Zwraca (lista_gałęzi, bieżąca_gałąź) dla danego katalogu.
    Jeśli katalog nie jest repozytorium git, zwraca (['main'], 'main').
    """
    git_dir = os.path.join(folder_path, ".git")
    if not os.path.exists(git_dir):
        return (["main"], "main")

    branches_raw = os.popen(
        f"git -C {folder_path!r} branch --format='%(refname:short)' 2>/dev/null"
    ).read().strip()
    branches = [b.strip() for b in branches_raw.splitlines() if b.strip()]

    current = os.popen(
        f"git -C {folder_path!r} branch --show-current 2>/dev/null"
    ).read().strip()

    if not branches:
        branches = [current or "main"]

    return (branches, current or branches[0])


def _dump_branch(folder_path: str, root_dir: str, branch: str,
                 code_extensions: set, code_filenames: set,
                 doc_extensions: set, ignore_dirs: set, ignore_files: set) -> None:
    """
    Dla podanej gałęzi: generuje parę plików koval_code.txt i koval_docs.txt.
    """
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    code_filename = f"koval_code_{timestamp}.txt"
    doc_filename = f"koval_docs_{timestamp}.txt"
    code_output_path = os.path.join(root_dir, code_filename)
    doc_output_path  = os.path.join(root_dir, doc_filename)

    with open(code_output_path, "w", encoding="utf-8") as code_f, \
         open(doc_output_path,  "w", encoding="utf-8") as doc_f:

        # Nagłówek każdego pliku
        for fh, kind in [(code_f, "CODE"), (doc_f, "DOCS")]:
            fh.write("Project : koval\n")
            fh.write(f"Branch  : {branch}\n")
            fh.write(f"Created : {datetime.now().isoformat()}\n")
            fh.write(f"Type    : {kind}\n")
            fh.write("=" * 80 + "\n\n")

        for root, dirs, files in os.walk(folder_path):
            dirs[:] = [d for d in dirs if d not in ignore_dirs]

            for file in sorted(files):
                if file in ignore_files or file.endswith(("_code.txt", "_docs.txt")) or "_code_" in file or "_docs_" in file:
                    continue

                file_path = os.path.join(root, file)
                rel_path  = os.path.relpath(file_path, root_dir)
                ext       = os.path.splitext(file)[1].lower()

                is_code = (ext in code_extensions) or (file in code_filenames)
                is_doc  = ext in doc_extensions

                target_f = code_f if is_code else (doc_f if is_doc else None)

                if target_f:
                    try:
                        with open(file_path, "r", encoding="utf-8") as rf:
                            content = rf.read()
                        target_f.write(f"\n{'=' * 80}\n")
                        target_f.write(f"FILE: {rel_path}\n")
                        target_f.write(f"{'=' * 80}\n\n")
                        target_f.write(content)
                        target_f.write(f"\n\n{'---' * 20}\n")
                    except Exception as e:
                        target_f.write(f"\n[ERROR READING {rel_path}: {e}]\n")

    print(f"  [koval/{branch}] -> {code_filename}")
    print(f"  [koval/{branch}] -> {doc_filename}")


def merge_files():
    root_dir   = "/my_data/KOVAL"
    subfolders = ["."]

    code_extensions = {".rs", ".toml", ".lock", ".yaml", ".yml", ".json", ".sql", ".sh"}
    code_filenames  = {
        "Dockerfile", "Dockerfile.test", "docker-compose.yml", "docker-compose.test.yml",
        "koval.toml", ".env", ".env.example"
    }
    doc_extensions  = {".md", ".txt"}

    ignore_dirs  = {
        ".git", "target", "test-artifacts", "test-repos", "koval-artifacts",
        "__pycache__", "venv", ".pytest_cache"
    }
    ignore_files = {"merge_files.py", "koval_code.txt", "koval_docs.txt"}

    for folder in subfolders:
        folder_path = os.path.join(root_dir, folder)
        if not os.path.exists(folder_path):
            print(f"[SKIP] {folder} — katalog nie istnieje")
            continue

        branches, current_branch = _get_branches(folder_path)
        git_available = os.path.exists(os.path.join(folder_path, ".git"))

        print(f"\n[{folder}] Gałęzie: {branches}  (bieżąca: {current_branch})")

        for branch in branches:
            # Checkout do gałęzi jeśli to git i gałąź jest inna niż bieżąca
            if git_available and branch != current_branch:
                ret = os.system(
                    f"git -C {folder_path!r} checkout -q {branch} 2>/dev/null"
                )
                if ret != 0:
                    print(f"  [WARN] Nie można przełączyć na {branch}, pomijam.")
                    continue

            _dump_branch(
                folder_path=folder_path,
                root_dir=root_dir,
                branch=branch,
                code_extensions=code_extensions,
                code_filenames=code_filenames,
                doc_extensions=doc_extensions,
                ignore_dirs=ignore_dirs,
                ignore_files=ignore_files,
            )

        # Wróć do oryginalnej gałęzi po przetworzeniu wszystkich gałęzi projektu
        if git_available and current_branch:
            os.system(f"git -C {folder_path!r} checkout -q {current_branch} 2>/dev/null")

    print(f"\nMerge zakończony. Pliki wygenerowane w: {root_dir}")


if __name__ == "__main__":
    merge_files()
