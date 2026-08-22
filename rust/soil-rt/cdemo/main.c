/* The embeddability proof (impl plan §4 step 8, plan 01 exit
 * criterion): construct values through the C ABI, round-trip them
 * through canonical JSON, exercise the error path, and tear down with
 * zero live allocations. */

#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "soil_rt.h"

int main(void) {
    SoilRuntime *rt = soil_init();
    assert(rt != NULL);
    SoilError *err = NULL;

    const char *types =
        "[{\"name\":\"SummaryRow\",\"strategy\":{\"tag\":\"Structural\"},"
        "\"body\":{\"tag\":\"Record\",\"value\":["
        "{\"name\":\"text\",\"shape\":{\"tag\":\"Utf8\"},\"ignored\":null},"
        "{\"name\":\"id\",\"shape\":{\"tag\":\"I64\"},\"ignored\":null}]}}]";
    assert(soil_register_types(rt, types, &err) == 0);

    uint32_t id = soil_type_lookup(rt, "SummaryRow");
    assert(id != UINT32_MAX);

    /* Construct {"text":"hello","id":7} through the ABI. */
    SoilValue *text = soil_utf8_new("hello", &err);
    SoilValue *num = soil_i64_new(7);
    assert(text != NULL && num != NULL);
    const SoilValue *const fields[] = {text, num};
    SoilValue *row = soil_record_new(rt, id, fields, 2, &err);
    assert(row != NULL);

    /* show: canonical JSON, byte-exact. */
    char *shown = soil_show(rt, row, &err);
    assert(shown != NULL);
    assert(strcmp(shown, "{\"text\":\"hello\",\"id\":7}") == 0);

    /* decode(show(v)) is eq to v. */
    SoilValue *decoded =
        soil_decode(rt, "{\"tag\":\"Named\",\"value\":\"SummaryRow\"}", shown, &err);
    assert(decoded != NULL);
    assert(soil_eq(rt, row, decoded, &err) == 1);

    /* Accessors read through the handle. */
    int64_t out = 0;
    assert(soil_i64_get(num, &out, &err) == 0 && out == 7);

    /* The error path is structured, not a crash. */
    SoilValue *bad = soil_decode(rt, "{\"tag\":\"I64\"}", "1.5", &err);
    assert(bad == NULL && err != NULL);
    char *message = soil_error_message(err);
    printf("cdemo: expected error: %s\n", message);
    soil_string_free(message);
    soil_error_free(err);
    err = NULL;

    /* Everything freed: the debug counter must read zero. */
    soil_string_free(shown);
    soil_value_free(decoded);
    soil_value_free(row);
    soil_value_free(num);
    soil_value_free(text);
    assert(soil_debug_live_values() == 0);

    soil_teardown(rt);
    printf("cdemo: ok\n");
    return 0;
}
