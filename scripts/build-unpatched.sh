#!/bin/bash

LATEST_PATH="./out/latest/"

# we can also pin to 132.0.6834.159 as a working version
# npx @puppeteer/browsers install chrome-headless-shell@stable
PLAYWRIGHT_BROWSERS_PATH=./chrome-headless-shell npx playwright install chromium --only-shell --with-deps

mkdir -p "$LATEST_PATH"

# move contents to out
mv ./chrome-headless-shell/*/* "$LATEST_PATH"

# strip away name
for dir in "$LATEST_PATH"/chrome-headless-shell-*; do
    # Check if it is actually a directory
    if [[ -d "$dir" ]]; then
        # Extract the base name without the "-$arc" part
        new_name="$LATEST_PATH/chrome-headless-shell"
        echo "Renaming directory $dir to $new_name"
        mv "$dir" "$new_name"

        # Check if the binary exists in the newly renamed directory
        binary_path="$new_name/headless_shell"
        new_binary_name="$new_name/headless-shell"

        if [[ -f "$binary_path" ]]; then
            echo "Renaming binary $binary_path to $new_binary_name"
            mv "$binary_path" "$new_binary_name"
        else
            echo "Binary $binary_path not found."
        fi
        
        break
    fi
done

# Remove the now-empty chrome-headless-shell directory
rm -R ./chrome-headless-shell

cd $LATEST_PATH

mv chrome-headless-shell ../

cd ../

mv chrome-headless-shell headless-shell
mv headless-shell ./latest

cd latest

mv chrome-linux headless-shell

cd headless-shell

mv headless_shell headless-shell

cd ..
