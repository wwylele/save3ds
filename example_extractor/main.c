#include <stdio.h>
#include <assert.h>
#include "../libsave3ds_c/include/save3ds_c.h"

void traverse(int indent, SaveDir *dir)
{
    EntryList *list = save3ds_save_dir_list_sub_dir(dir);
    assert(list);
    unsigned len = save3ds_entry_list_len(list);
    for (unsigned i = 0; i < len; ++i)
    {
        char name[17] = {};
        unsigned ino;
        save3ds_entry_list_get(list, i, name, &ino);
        for (int k = 0; k < indent; ++k)
            printf(" ");
        printf("+%s\n", name);
        SaveDir *sub = save3ds_save_dir_open_sub_dir(dir, name);
        assert(sub);
        traverse(indent + 1, sub);
        save3ds_save_dir_release(sub);
    }
    save3ds_entry_list_release(list);

    list = save3ds_save_dir_list_sub_file(dir);
    assert(list);
    len = save3ds_entry_list_len(list);
    for (unsigned i = 0; i < len; ++i)
    {
        char name[17] = {};
        unsigned ino;
        save3ds_entry_list_get(list, i, name, &ino);
        for (int k = 0; k < indent; ++k)
            printf(" ");
        printf("-%s\n", name);
    }
    save3ds_entry_list_release(list);
}

int main(int *argc, char **argv)
{
    Resource *resource = save3ds_resource_create(NULL, NULL, NULL, NULL);
    assert(resource);
    SaveData *save = save3ds_open_bare_save(resource, argv[1]);
    assert(save);
    SaveDir *root = save3ds_save_dir_open_root(save);
    assert(root);
    traverse(0, root);
    save3ds_save_dir_release(root);
    save3ds_save_release(save);
    save3ds_resource_release(resource);
}
